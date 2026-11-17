//! Real kernel locks and processes distinguish shutdown requests from stop proof.
use super::*;
use ora_node_db::{SessionStopHost, SessionStopKind, SessionStopProof};
use ora_node_protocol::{NodeId, NodeIncarnationId, NodeRuntimeIdentity};
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec};
use pretty_assertions::assert_eq;
use std::os::unix::fs::PermissionsExt;
use tokio::io::{AsyncBufReadExt, BufReader};

type TestResult = Result<(), Box<dyn std::error::Error>>;

#[tokio::test]
async fn stop_wait_keeps_original_bound_and_does_not_treat_request_failure_as_proof() -> TestResult
{
    use ora_node_db::{NodeDatabase, NodeIdentity, SessionStopChange, SessionStopState};
    use ora_node_protocol::*;
    let (home, files, identity) = fixture()?;
    let mut db = NodeDatabase::open(
        &home.path().join("db"),
        NodeIdentity::Require(NodeId::new("node")),
    )?;
    let input: StartAgentSessionMessage = serde_json::from_value(serde_json::json!({
        "protocol_version":CURRENT_PROTOCOL_VERSION,"operation_id":"op","execution_id":"session",
        "payload":{"spec":{"node_id":"node","agent_plugin_id":"official/agent","agent_plugin_version":"1.0.0","checkout_execution_id":"checkout",
        "git_identity":{"name":"Fixture","email":"fixture@example.test"},"initial_turn":{"turn_id":"initial","content":[{"type":"text","text":"hello"}]}}}
    }))?;
    db.accept_session(&input)?;
    db.start_session(&input, &NodeIncarnationId::new("original"))?;
    let journal = db.session_journal()?;
    files.create(&identity.attempt)?;
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
    files.create(&identity.attempt)?;
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

#[tokio::test]
async fn plugin_and_each_host_child_share_one_attempt_while_siblings_stay_running() -> TestResult {
    let (_home, files, identity) = fixture()?;
    files.create(&identity.attempt)?;
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
            Some(SessionStopProof::LockAndProcesses)
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
    files.create(&identity.attempt)?;
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
    files.create(&identity.attempt)?;
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
