//! Drive the configured Node replacement callback with a supervisor whose retirement we control.
use super::{TestResult, fixture, journal};
use crate::session::{
    AgentSessions, SessionConfig, SessionWorkload, host::IgnoredStatus, mcp,
    process::ModelProcessIsolation, stop, workload::SessionPlacement,
};
use ora_agent_runtime::SessionSetup;
use ora_node_db::{SessionStopPackage, SessionStopProof, SessionStopState};
use ora_node_protocol::{GitIdentity, PluginId, PluginVersion};
use ora_plugin_lifecycle::{GenerationTaps, PluginLifecycle, PluginLifecycleConfig};
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec};
use ora_utils::http::ReqwestDownloader;
use pretty_assertions::assert_eq;
use std::{os::unix::process::ExitStatusExt, time::Duration};
use tokio::sync::oneshot;

/// The old supervisor addresses plugins by identity, so it must finish before reuse.
#[tokio::test]
async fn replacement_waits_for_retirement_before_registering_the_successor() -> TestResult {
    let (home, _, mut old) = fixture()?;
    let cwd = home.path();
    let journal = journal(cwd)?;
    let plugin = ora_domain::PluginId::parse("official/agent")?;
    old.packages.push(SessionStopPackage {
        plugin_id: PluginId::new(plugin.to_string()),
        plugin_version: PluginVersion::new("1.0.0"),
        view_key: None,
    });
    let catalog = crate::PluginInstaller::new(
        cwd.into(),
        ReqwestDownloader::new(Default::default()),
        /*host_target*/ None,
    )
    .catalog();
    let (setup, _) = mcp::NodeSessionMcp::prepare(&catalog, vec![], cwd)?;
    let config = SessionConfig {
        stop: Default::default(),
        home_directory: cwd.into(),
        deno_path: cwd.join("unused-deno"),
        timezone: chrono_tz::UTC,
        agent_ready_timeout: Duration::from_secs(5),
        model_proxy: None,
        workload: SessionWorkload::Shared,
    };
    let sessions = AgentSessions::new(
        config,
        old.node.clone(),
        journal.clone(),
        journal.clone(),
        catalog,
        None::<
            crate::revision::RevisionRestorer<crate::managed::DirectGit, crate::managed::DirectGit>,
        >,
    );
    let shared = &sessions.shared;
    let placement = SessionPlacement::Shared;
    let isolation = ModelProcessIsolation::Ambient;
    let spawner = stop::prepare(
        &journal,
        cwd,
        old.clone(),
        isolation.spawner(placement.identity()),
    )?;
    let launcher = placement.launcher(
        &GitIdentity {
            name: "Fixture".into(),
            email: "fixture@example.test".into(),
        },
        cwd,
        Default::default(),
        isolation,
        spawner.clone(),
    );
    let lifecycle = PluginLifecycle::open(
        PluginLifecycleConfig {
            data_directory: cwd.into(),
            deno_path: shared.config.deno_path.clone(),
        },
        launcher,
        IgnoredStatus,
        GenerationTaps::default(),
    )?;
    shared
        .controls
        .lock()
        .unwrap()
        .insert(old.execution.clone(), mcp::control::Control::new(&[]));
    shared.retained.lock().unwrap().insert(
        old.execution.clone(),
        stop::Retained {
            placement,
            isolation,
            leases: vec![],
            spawner: spawner.clone(),
        },
    );
    let setup = mcp::configure(setup, shared, &old.execution, lifecycle, plugin, cwd.into());
    let child = spawner.spawn(ProcessSpec::new("sleep").arg("60"))?;
    let records = || journal.session_stop_records(Some(&old.execution));
    let (entered, retiring) = oneshot::channel();
    let (release, gate) = oneshot::channel();
    let replacement = setup.replace_after_retirement(async {
        let [record] = records().unwrap().try_into().unwrap();
        assert_eq!(record.identity, old);
        assert!(
            record.first_stop_at_ms.is_some(),
            "stop request must precede retirement"
        );
        assert!(!spawner.is_open());
        // SIGKILL delivery is asynchronous. Observe the sole group member's exit while
        // retirement is suspended, before stop confirmation can signal it again.
        assert_eq!(child.wait().await.unwrap().signal(), Some(libc::SIGKILL));
        entered.send(()).unwrap();
        gate.await.unwrap();
    });
    tokio::pin!(replacement);
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            biased;
            result = &mut replacement => panic!("replacement finished before retirement: {result:?}"),
            result = retiring => result.expect("replacement dropped the retirement future"),
        }
    }).await?;
    assert_eq!(records()?.len(), 1);
    assert_eq!(spawner.identity(), old);
    release.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(5), replacement).await??;
    let records = records()?;
    assert_eq!(records.len(), 2);
    let next = spawner.identity();
    assert_ne!(next.attempt, old.attempt);
    assert!(
        records
            .iter()
            .any(|r| r.identity == old && r.proof == Some(SessionStopProof::LockAndProcesses))
    );
    assert!(
        records
            .iter()
            .any(|r| r.identity == next && r.state == SessionStopState::Prepared)
    );
    Ok(())
}
