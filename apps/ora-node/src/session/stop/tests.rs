//! Real kernel locks and processes distinguish shutdown requests from stop proof.
mod marks;
mod replacement;

use super::*;
use ora_node_db::{SessionStopHost, SessionStopKind, SessionStopProof};
use ora_node_protocol::{NodeId, NodeIncarnationId, NodeRuntimeIdentity};
use ora_process::{ManagedProcess, ProcessIdentity, ProcessSpawner, ProcessSpec};
use pretty_assertions::assert_eq;
use std::os::unix::fs::PermissionsExt;
use tokio::io::{AsyncBufReadExt, BufReader};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn stop_wait_keeps_original_bound_and_does_not_treat_request_failure_as_proof() -> TestResult
{
    use ora_node_db::{SessionStopChange, SessionStopState};
    let (home, files, identity) = fixture()?;
    let journal = journal(home.path())?;
    files.create(&identity.attempt, ProcessIdentity::Inherit)?;
    let descriptor = files.hold(&identity.attempt)?;
    journal.record_session_stop(&identity, now_ms())?;
    let config = SessionStopConfig {
        confirmation_timeout_ms: 500,
        ..Default::default()
    };
    request(&journal, &identity.execution, &config)?;
    let original_bound = journal.session_stop_records(None)?[0].bound_at_ms;
    assert!(!confirm(&journal, home.path(), &config, &identity.execution, false).await?);
    assert_eq!(
        journal.session_stop_records(None)?[0].state,
        SessionStopState::Stopping
    );
    let release = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(descriptor);
    });
    let began = tokio::time::Instant::now();
    assert!(
        await_plugin_stop(&journal, home.path(), &config, &identity.execution, async {
            Err::<(), _>(std::io::Error::other("injected lifecycle stop failure"))
        })
        .await?
    );
    assert!(began.elapsed() >= Duration::from_millis(90));
    release.await?;
    assert_eq!(
        journal.session_stop_records(None)?[0].bound_at_ms,
        original_bound
    );
    journal.change_session_stop(&identity.attempt, SessionStopChange::ResourcesReleased)?;
    Ok(())
}

#[test]
fn foreign_or_missing_host_identity_cannot_turn_a_boot_change_into_stop_proof() -> TestResult {
    let (_home, files, mut identity) = fixture()?;
    files.create(&identity.attempt, ProcessIdentity::Inherit)?;
    identity.host.boot_id = "old-boot".into();
    identity.host.trusted_host_id = Some("old-host".into());
    for host in [Some("other-host".into()), None, Some(String::new())] {
        let current = SessionStopHost {
            boot_id: "new-boot".into(),
            trusted_host_id: host,
        };
        assert_eq!(
            files.probe(&identity, &current, false),
            (StopEvidence::Unknown, None)
        );
    }
    Ok(())
}

/// Use the production journal so assertions cover the durable proof and original deadline.
fn journal(home: &Path) -> Result<ora_node_db::SessionJournal, Box<dyn std::error::Error>> {
    use ora_node_db::{NodeDatabase, NodeIdentity};
    use ora_node_protocol::*;
    let mut db = NodeDatabase::open(&home.join("db"), NodeIdentity::Require(NodeId::new("node")))?;
    let input: StartAgentSessionMessage = serde_json::from_value(serde_json::json!({
        "protocol_version":CURRENT_PROTOCOL_VERSION,"operation_id":"op","execution_id":"session",
        "payload":{"spec":{"node_id":"node","agent_plugin_id":"official/agent","agent_plugin_version":"1.0.0","checkout_execution_id":"checkout",
        "git_identity":{"name":"Fixture","email":"fixture@example.test"},"initial_turn":{"turn_id":"initial","content":[{"type":"text","text":"hello"}]}}}
    }))?;
    db.accept_session(&input)?;
    db.start_session(&input, &NodeIncarnationId::new("original"))?;
    Ok(db.session_journal()?)
}

/// A successful lifecycle return cannot confirm an attempt with a live lock holder.
#[tokio::test]
async fn successful_cleanup_with_held_lock_leaves_attempt_unconfirmed() -> TestResult {
    let (home, files, identity) = fixture()?;
    let journal = journal(home.path())?;
    files.create(&identity.attempt, ProcessIdentity::Inherit)?;
    let _held = files.hold(&identity.attempt)?;
    journal.record_session_stop(&identity, now_ms())?;
    let config = SessionStopConfig {
        confirmation_timeout_ms: 1,
        ..Default::default()
    };
    request(&journal, &identity.execution, &config)?;
    let [original] = journal.session_stop_records(None)?.try_into().unwrap();
    assert!(
        !await_plugin_stop(&journal, home.path(), &config, &identity.execution, async {
            Ok::<(), std::io::Error>(())
        })
        .await?
    );
    assert_eq!(
        journal.session_stop_records(None)?,
        vec![ora_node_db::SessionStopRecord {
            state: SessionStopState::StopUnconfirmed,
            ..original
        }]
    );
    Ok(())
}

/// Every fixture has an owner-private independent Node home and a value-free attempt identity.
fn fixture()
-> Result<(tempfile::TempDir, StopFiles, SessionStopIdentity), Box<dyn std::error::Error>> {
    let home = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()?;
    let files = StopFiles::open(home.path())?;
    let identity = SessionStopIdentity {
        attempt: AttemptId::generate(),
        execution: ExecutionId::new("session"),
        checkout: ExecutionId::new("checkout"),
        node: NodeRuntimeIdentity {
            node_id: NodeId::new("node"),
            incarnation_id: NodeIncarnationId::new("original"),
        },
        host: SessionStopHost::current_untrusted(),
        kind: SessionStopKind::Agent,
        packages: vec![],
    };
    Ok((home, files, identity))
}

/// Lifecycle acknowledgement may need process exit; the durable owner must signal first.
#[tokio::test]
async fn stop_confirmation_terminates_plugin_and_cli_while_lifecycle_waits() -> TestResult {
    let (home, files, identity) = fixture()?;
    let journal = journal(home.path())?;
    let spawner = prepare(
        &journal,
        home.path(),
        identity.clone(),
        TokioProcessSpawner::new().without_reaper_registration(),
    )?;
    let plugin = spawner.spawn(ProcessSpec::new("sleep").arg("60"))?;
    let cli = spawner.spawn(ProcessSpec::new("sleep").arg("60"))?;
    let config = SessionStopConfig::default();
    request(&journal, &identity.execution, &config)?;
    spawner.close();
    let [original] = journal.session_stop_records(None)?.try_into().unwrap();
    let mut lifecycle_finished = false;
    let confirmed = tokio::time::timeout(
        Duration::from_millis(750),
        await_plugin_stop(&journal, home.path(), &config, &identity.execution, async {
            plugin.wait().await?;
            cli.wait().await?;
            // Keep lifecycle cleanup pending beyond process exit and durable confirmation.
            while journal.session_stop_records(None)?[0].state != SessionStopState::Confirmed {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            lifecycle_finished = true;
            Ok::<(), Box<dyn std::error::Error>>(())
        }),
    )
    .await??;
    assert!(confirmed && lifecycle_finished);
    let records = journal.session_stop_records(None)?;
    let confirmed_at = records[0]
        .confirmed_at_ms
        .ok_or("missing confirmation time")?;
    assert_eq!(
        records,
        vec![ora_node_db::SessionStopRecord {
            state: SessionStopState::Confirmed,
            proof: Some(SessionStopProof::LockProcessesAndMarks),
            confirmed_at_ms: Some(confirmed_at),
            ..original
        }]
    );
    assert!(plugin.try_wait()?.is_some());
    assert!(cli.try_wait()?.is_some());
    assert_eq!(
        files.probe(&identity, &identity.host, false),
        (
            StopEvidence::Stopped,
            Some(SessionStopProof::LockProcessesAndMarks)
        )
    );
    Ok(())
}

/// Replacement cleanup waits on real children whose original groups must be signalled first.
#[tokio::test]
async fn replacement_signals_old_groups_before_lifecycle_cleanup() -> TestResult {
    let (_home, files, identity) = fixture()?;
    files.create(&identity.attempt, ProcessIdentity::Inherit)?;
    let spawner = SessionAttemptSpawner::new(
        TokioProcessSpawner::new().without_reaper_registration(),
        identity.clone(),
        files,
    )?;
    let plugin = spawner.spawn(ProcessSpec::new("sleep").arg("60"))?;
    let cli = spawner.spawn(ProcessSpec::new("sleep").arg("60"))?;
    assert!(spawner.is_open());
    tokio::time::timeout(
        Duration::from_millis(750),
        spawner.stop_for_replacement(&identity.host, async {
            assert!(!spawner.is_open());
            plugin.wait().await?;
            cli.wait().await?;
            Ok::<(), std::io::Error>(())
        }),
    )
    .await??;
    Ok(())
}

#[tokio::test]
async fn plugin_and_each_host_child_share_one_attempt_while_siblings_stay_running() -> TestResult {
    let (_home, files, identity) = fixture()?;
    files.create(&identity.attempt, ProcessIdentity::Inherit)?;
    let inner = TokioProcessSpawner::new().without_reaper_registration();
    let spawner = SessionAttemptSpawner::new(inner.clone(), identity.clone(), files.clone())?;
    let plugin = spawner.spawn(ProcessSpec::new("sleep").arg("60"))?;
    let cli = spawner.spawn(ProcessSpec::new("sleep").arg("60"))?;
    let sibling = inner.spawn(ProcessSpec::new("sleep").arg("60"))?;
    spawner.close();
    assert!(spawner.spawn(ProcessSpec::new("true")).is_err());
    plugin.kill().await?;
    plugin.wait().await?;
    assert_eq!(
        files.probe(&identity, &identity.host, false).0,
        StopEvidence::Running
    );
    files.terminate(&identity, &identity.host);
    cli.wait().await?;
    // A concurrent fork can retain CLOEXEC descriptors until its exec; the owner polls proof.
    tokio::time::timeout(Duration::from_secs(2), async {
        while files.probe(&identity, &identity.host, false).0 == StopEvidence::Running {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert_eq!(
        files.probe(&identity, &identity.host, false),
        (
            StopEvidence::Stopped,
            Some(SessionStopProof::LockProcessesAndMarks)
        )
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(30), sibling.wait())
            .await
            .is_err()
    );
    sibling.kill().await?;
    sibling.wait().await?;
    Ok(())
}

#[tokio::test]
async fn live_direct_child_that_closes_its_descriptor_is_not_stop_proof() -> TestResult {
    let (_home, files, identity) = fixture()?;
    files.create(&identity.attempt, ProcessIdentity::Inherit)?;
    let spawner = SessionAttemptSpawner::new(
        TokioProcessSpawner::new().without_reaper_registration(),
        identity.clone(),
        files.clone(),
    )?;
    let mut process=spawner.spawn(ProcessSpec::new("python3").args(["-c", "import os,time\nfor name in os.listdir('/proc/self/fd'):\n fd=int(name)\n if fd>2:\n  try: os.close(fd)\n  except OSError: pass\nprint('ready',flush=True)\ntime.sleep(60)"]))?;
    let output = process.take_stdout().ok_or("missing stdout")?;
    let mut line = String::new();
    tokio::time::timeout(
        Duration::from_secs(5),
        BufReader::new(output).read_line(&mut line),
    )
    .await??;
    assert_eq!(line.trim(), "ready");
    spawner.close();
    assert_eq!(
        files.probe(&identity, &identity.host, false).0,
        StopEvidence::Running
    );
    process.kill().await?;
    process.wait().await?;
    // Concurrent forks can briefly retain the lock descriptor before exec closes it.
    tokio::time::timeout(Duration::from_secs(2), async {
        while files.probe(&identity, &identity.host, false).0 == StopEvidence::Running {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    assert_eq!(
        files.probe(&identity, &identity.host, false).0,
        StopEvidence::Stopped
    );
    Ok(())
}

#[test]
fn a_durable_unstarted_lock_is_proof_but_missing_corrupt_or_foreign_boot_evidence_is_not()
-> TestResult {
    let (home, files, mut identity) = fixture()?;
    assert_eq!(
        files.probe(&identity, &identity.host, false).0,
        StopEvidence::Unknown
    );
    files.create(&identity.attempt, ProcessIdentity::Inherit)?;
    assert_eq!(
        files.probe(&identity, &identity.host, false).0,
        StopEvidence::Stopped
    );
    let original = identity.host.clone();
    identity.host.boot_id = "old-kernel".into();
    assert_eq!(
        files.probe(&identity, &original, false).0,
        StopEvidence::Unknown
    );
    assert_eq!(
        files.probe(&identity, &original, true).1,
        Some(SessionStopProof::FencedVolume)
    );
    identity.host.trusted_host_id = Some("physical-host".into());
    let mut current = original;
    current.trusted_host_id = Some("physical-host".into());
    assert_eq!(
        files.probe(&identity, &current, false).1,
        Some(SessionStopProof::HostReboot)
    );
    identity.host = current;
    std::fs::write(
        home.path()
            .join(format!("session-attempts/{}.lock", identity.attempt)),
        b"damaged",
    )?;
    assert_eq!(
        files.probe(&identity, &identity.host, false).0,
        StopEvidence::Unknown
    );
    identity.kind = SessionStopKind::LegacyAgent;
    assert_eq!(
        files.probe(&identity, &identity.host, false).0,
        StopEvidence::Unknown
    );
    Ok(())
}
