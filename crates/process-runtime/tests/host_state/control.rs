use super::*;
use ora_process_protocol::{
    DescendantPolicy, GuardianHostDisconnect, HostRunIntent, RunId, RunSpec,
};
use pretty_assertions::assert_eq;

/// Closing admission and accepting cancellation survive restart without dispatching any work.
#[test]
fn durable_control_seals_admission_but_preserves_duplicate_lookup() -> TestResult {
    let directory = directory()?;
    let path = directory.path().join("s");
    let mut host = HostState::create(&path)?;
    let scope = ScopeId::new();
    let original_scope = host.record_scope_intent(scope)?;
    let intent = HostRunIntent {
        scope,
        run: RunId::new(),
        spec: RunSpec::new("/bin/true", directory.path(), DescendantPolicy::WaitForAll),
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    host.record_run_intent(intent.clone())?;
    assert!(!host.run_stop_requested(intent.run)?);
    host.request_run_stop(intent.run)?;
    host.request_run_stop(intent.run)?;
    assert!(host.run_stop_requested(intent.run)?);
    host.request_scope_close(scope)?;
    host.request_scope_close(scope)?;
    assert_eq!(host.record_run_intent(intent.clone())?, intent);
    let mut later = intent.clone();
    later.run = RunId::new();
    assert!(host.record_run_intent(later.clone()).is_err());
    assert_eq!(host.run_intent(later.run)?, None);
    assert!(host.request_run_stop(later.run).is_err());
    assert!(host.request_scope_close(ScopeId::new()).is_err());
    drop(host);
    let host = recover(&path)?;
    assert!(host.run_stop_requested(intent.run)?);
    assert!(host.scope_close_requested(scope)?);
    assert_eq!(host.scope_intents()?, vec![original_scope]);
    assert_eq!(host.guardian_access(scope)?, None);
    Ok(())
}

/// A v4 migration preserves every Run and creates empty control journals, never guessed cancellation.
#[test]
fn version_four_upgrade_and_failed_control_write_preserve_intent() -> TestResult {
    let directory = directory()?;
    let path = directory.path().join("s");
    let mut host = HostState::create(&path)?;
    let scope = ScopeId::new();
    host.record_scope_intent(scope)?;
    let intent = HostRunIntent {
        scope,
        run: RunId::new(),
        spec: RunSpec::new("/bin/true", directory.path(), DescendantPolicy::WaitForAll),
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    host.record_run_intent(intent.clone())?;
    drop(host);
    let connection = rusqlite::Connection::open(path.join("host.sqlite"))?;
    connection.execute_batch(
        "DROP TABLE run_observations; DROP TABLE scope_observations; DROP TABLE run_stop_intents; DROP TABLE scope_close_intents; PRAGMA user_version=4;",
    )?;
    drop(connection);
    let mut host = recover(&path)?;
    assert_eq!(host.run_intents()?, vec![intent.clone()]);
    assert!(!host.run_stop_requested(intent.run)?);
    let injector = rusqlite::Connection::open(path.join("host.sqlite"))?;
    injector.execute_batch("CREATE TRIGGER reject_close BEFORE INSERT ON scope_close_intents BEGIN SELECT RAISE(ABORT, 'fixture'); END;")?;
    assert!(host.request_scope_close(scope).is_err());
    assert!(!host.scope_close_requested(scope)?);
    injector.execute_batch("DROP TRIGGER reject_close;")?;
    host.request_scope_close(scope)?;
    assert!(host.run_stop_requested(intent.run)?);
    drop(host);
    assert!(recover(&path)?.run_stop_requested(intent.run)?);
    Ok(())
}

/// Orphaned cancellation records cannot silently disappear into a newly advanced host epoch.
#[test]
fn corrupt_control_references_block_recovery_read_only() -> TestResult {
    let directory = directory()?;
    for table in ["run_stop_intents", "scope_close_intents"] {
        let path = directory.path().join(table);
        drop(HostState::create(&path)?);
        let injector = rusqlite::Connection::open(path.join("host.sqlite"))?;
        injector.execute_batch("PRAGMA foreign_keys=OFF;")?;
        injector.execute(&format!("INSERT INTO {table} VALUES ('invalid')"), [])?;
        drop(injector);
        let original = fs::read(path.join("host.sqlite"))?;
        assert!(HostState::recover(&path).is_err());
        assert_eq!(fs::read(path.join("host.sqlite"))?, original);
    }
    Ok(())
}
