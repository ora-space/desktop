use super::*;
use ora_process_protocol::{
    DescendantPolicy, GuardianHostDisconnect, HostRunIntent, RunId, RunSpec,
};
use pretty_assertions::assert_eq;

/// An immutable intent can be discovered after recovery without a process or Scope directory existing.
#[test]
fn run_intents_preserve_identity_parameters_and_scope_across_recovery() -> TestResult {
    let directory = directory()?;
    let path = directory.path().join("s");
    let mut host = HostState::create(&path)?;
    let scope = ScopeId::new();
    host.record_scope_intent(scope)?;
    let mut spec = RunSpec::new("/bin/sh", directory.path(), DescendantPolicy::WaitForAll);
    spec.args = ["-c", "printf never-executed"].map(Into::into).to_vec();
    // Persistence uses native OS encoding for every path/argument/environment value.
    use std::os::unix::ffi::OsStringExt;
    let native = std::ffi::OsString::from_vec(vec![b'x', 0xff]);
    spec.cwd = directory.path().join(&native);
    spec.args.push(native.clone());
    spec.env.insert(native.clone(), native);
    let intent = HostRunIntent {
        scope,
        run: RunId::new(),
        spec,
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    assert_eq!(host.record_run_intent(intent.clone())?, intent);
    assert_eq!(host.record_run_intent(intent.clone())?, intent);
    let mut conflicting = intent.clone();
    conflicting.spec.args.push("changed".into());
    assert!(host.record_run_intent(conflicting).is_err());
    let other_scope = ScopeId::new();
    host.record_scope_intent(other_scope)?;
    let mut conflicting = intent.clone();
    conflicting.scope = other_scope;
    assert!(host.record_run_intent(conflicting).is_err());
    let mut absent_scope = intent.clone();
    absent_scope.run = RunId::new();
    absent_scope.scope = ScopeId::new();
    assert!(host.record_run_intent(absent_scope.clone()).is_err());
    assert_eq!(host.run_intent(absent_scope.run)?, None);
    let mut second = intent.clone();
    second.run = RunId::new();
    second.scope = other_scope;
    host.record_run_intent(second.clone())?;
    let mut expected = vec![intent.clone(), second];
    expected.sort_by_key(|intent| intent.run);
    let binding = host.binding();
    let inode = fs::metadata(path.join("host.lock"))?.ino();
    drop(host);
    let mut host = recover(&path)?;
    assert_eq!(host.run_intents()?, expected);
    assert_eq!(host.run_intent(intent.run)?, Some(intent.clone()));
    assert_eq!(host.record_run_intent(intent.clone())?, intent);
    assert_eq!(host.binding().epoch.get(), binding.epoch.get() + 1);
    assert_eq!(fs::metadata(path.join("host.lock"))?.ino(), inode);
    assert_eq!(fs::read_dir(path.join("scopes"))?.count(), 0);
    Ok(())
}

/// The exact v3 schema upgrades additively without touching a legacy guardian's owned files.
#[test]
fn version_three_upgrade_does_not_invent_historical_run_intents() -> TestResult {
    let directory = directory()?;
    let path = directory.path().join("s");
    let mut host = HostState::create(&path)?;
    let scope = ScopeId::new();
    let intent = host.record_scope_intent(scope)?;
    drop(host);
    let legacy = rusqlite::Connection::open(path.join("host.sqlite"))?;
    legacy.execute_batch("DROP TABLE run_observations; DROP TABLE scope_observations; DROP TABLE run_stop_intents; DROP TABLE scope_close_intents; DROP TABLE run_intents; PRAGMA user_version=3;")?;
    legacy.execute(
        "INSERT INTO guardian_launches VALUES (?1, 'launch_unknown')",
        [scope.to_string()],
    )?;
    drop(legacy);
    let scope_path = path.join("scopes").join(scope.to_string());
    fs::create_dir(&scope_path)?;
    fs::set_permissions(&scope_path, fs::Permissions::from_mode(/*mode*/ 0o700))?;
    fs::write(
        scope_path.join("guardian.sqlite"),
        b"guardian-owned data, not a host database",
    )?;
    let mut host = recover(&path)?;
    assert_eq!(host.run_intents()?, Vec::<HostRunIntent>::new());
    assert_eq!(
        host.guardian_access(scope)?,
        Some(ora_process_protocol::GuardianAccess {
            scope_dir: scope_path.clone(),
            intent
        })
    );
    assert_eq!(
        fs::read(scope_path.join("guardian.sqlite"))?,
        b"guardian-owned data, not a host database"
    );
    let run = HostRunIntent {
        scope,
        run: RunId::new(),
        spec: RunSpec::new("/bin/true", directory.path(), DescendantPolicy::WaitForAll),
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    host.record_run_intent(run.clone())?;
    drop(host);
    assert_eq!(recover(&path)?.run_intents()?, vec![run]);
    Ok(())
}

/// Storage refusal and an oversized dispatch leave no accepted responsibility behind.
#[test]
fn failed_intent_commit_and_oversized_message_are_not_accepted() -> TestResult {
    let directory = directory()?;
    let path = directory.path().join("s");
    let mut host = HostState::create(&path)?;
    let scope = ScopeId::new();
    host.record_scope_intent(scope)?;
    let mut intent = HostRunIntent {
        scope,
        run: RunId::new(),
        spec: RunSpec::new("/bin/true", directory.path(), DescendantPolicy::WaitForAll),
        host_disconnect: GuardianHostDisconnect::KeepRunning,
    };
    let injector = rusqlite::Connection::open(path.join("host.sqlite"))?;
    injector.execute_batch("CREATE TRIGGER reject_run BEFORE INSERT ON run_intents BEGIN SELECT RAISE(ABORT, 'fixture'); END;")?;
    assert!(host.record_run_intent(intent.clone()).is_err());
    assert_eq!(host.run_intents()?, Vec::<HostRunIntent>::new());
    injector.execute_batch("DROP TRIGGER reject_run;")?;
    intent
        .spec
        .args
        .push("x".repeat(ora_process_protocol::GUARDIAN_MAX_FRAME).into());
    assert!(host.record_run_intent(intent.clone()).is_err());
    assert_eq!(host.run_intent(intent.run)?, None);
    intent.spec.args.clear();
    assert_eq!(host.record_run_intent(intent.clone())?, intent);
    Ok(())
}

/// Corrupt payloads, indexes and scope references fail before advancing the host epoch.
#[test]
fn corrupt_run_intents_block_recovery_without_resetting_state() -> TestResult {
    let directory = directory()?;
    for statement in [
        "UPDATE run_intents SET intent=x'c0'",
        "UPDATE run_intents SET run='invalid'",
        "UPDATE run_intents SET scope='00000000-0000-0000-0000-000000000001'",
        "DELETE FROM scope_intents",
    ] {
        let fixture = tempfile::Builder::new()
            .prefix("j")
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(directory.path())?;
        let path = fixture.path().join("s");
        let mut host = HostState::create(&path)?;
        let scope = ScopeId::new();
        host.record_scope_intent(scope)?;
        host.record_run_intent(HostRunIntent {
            scope,
            run: RunId::new(),
            spec: RunSpec::new("/bin/true", fixture.path(), DescendantPolicy::WaitForAll),
            host_disconnect: GuardianHostDisconnect::KeepRunning,
        })?;
        drop(host);
        let injector = rusqlite::Connection::open(path.join("host.sqlite"))?;
        injector.execute_batch("PRAGMA foreign_keys=OFF;")?;
        injector.execute_batch(statement)?;
        drop(injector);
        let original = fs::read(path.join("host.sqlite"))?;
        assert!(HostState::recover(&path).is_err());
        assert_eq!(fs::read(path.join("host.sqlite"))?, original);
    }
    Ok(())
}
