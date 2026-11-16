//! Real kernel locks and processes distinguish shutdown requests from stop proof.
use super::*;
use ora_node_db::{SessionStopHost, SessionStopKind, SessionStopProof};
use ora_node_protocol::{NodeId, NodeIncarnationId, NodeRuntimeIdentity};
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec};
use pretty_assertions::assert_eq;
use std::os::unix::fs::PermissionsExt;
use tokio::io::{AsyncBufReadExt, BufReader};

type TestResult = Result<(), Box<dyn std::error::Error>>;

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
