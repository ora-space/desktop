use super::*;
use ora_process_protocol::{
    DescendantPolicy, GuardianHostDisconnect, HostCoordination, HostRunIntent, RunId, RunSpec,
};
use ora_process_runtime::HostCoordinator;
use pretty_assertions::assert_eq;

/// v5 migration keeps accepted control responsibility without inventing any guardian facts.
#[test]
fn version_five_upgrade_preserves_stop_and_close() -> TestResult {
    let directory = directory()?;
    let root = directory.path().join("s");
    let mut state = HostState::create(&root)?;
    let scope = ScopeId::new();
    state.record_scope_intent(scope)?;
    let intent = HostRunIntent {
        scope,
        run: RunId::new(),
        spec: RunSpec::new("/bin/true", directory.path(), DescendantPolicy::WaitForAll),
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    state.record_run_intent(intent.clone())?;
    state.request_run_stop(intent.run)?;
    state.request_scope_close(scope)?;
    drop(state);
    let old = rusqlite::Connection::open(root.join("host.sqlite"))?;
    old.execute_batch(
        "DROP TABLE run_observations; DROP TABLE scope_observations; PRAGMA user_version=5;",
    )?;
    drop(old);
    let host = HostCoordinator::new(recover(&root)?, directory.path().join("unused"));
    assert_eq!(
        host.query_run(intent.run)?,
        ora_process_protocol::HostRunView {
            scope,
            run: intent.run,
            stop_requested: true,
            last_observed: None,
            coordination: HostCoordination::Pending
        }
    );
    assert_eq!(
        host.query_scope(scope)?,
        ora_process_protocol::HostScopeView {
            scope,
            close_requested: true,
            last_observed: None,
            coordination: HostCoordination::Pending
        }
    );
    Ok(())
}

/// Unknown projection payloads are not silently dropped or interpreted as evidence of non-execution.
#[test]
fn corrupt_projection_blocks_recovery_without_changing_journal() -> TestResult {
    let directory = directory()?;
    for table in ["run_observations", "scope_observations"] {
        let root = directory.path().join(table);
        drop(HostState::create(&root)?);
        let injector = rusqlite::Connection::open(root.join("host.sqlite"))?;
        injector.execute_batch("PRAGMA foreign_keys=OFF;")?;
        injector.execute(
            &format!("INSERT INTO {table} VALUES ('00000000-0000-0000-0000-000000000001', x'c0')"),
            [],
        )?;
        drop(injector);
        let original = fs::read(root.join("host.sqlite"))?;
        assert!(HostState::recover(&root).is_err());
        assert_eq!(fs::read(root.join("host.sqlite"))?, original);
    }
    Ok(())
}
