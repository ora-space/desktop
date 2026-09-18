#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Barrier;
use std::time::{Duration, Instant};

use ora_process_protocol::{HostBinding, ScopeCreationIntent, ScopeId};
use ora_process_runtime::{HostState, ProcessStateError};
use pretty_assertions::assert_eq;

#[path = "host_state/control.rs"]
mod control;
#[path = "host_state/observations.rs"]
mod observations;
#[path = "host_state/runs.rs"]
mod runs;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Selects a trusted test fixture parent; the runtime receives only the resulting explicit path.
fn directory() -> Result<tempfile::TempDir, std::io::Error> {
    // Checkouts may be group-writable and /tmp is deliberately outside the admission policy.
    let parent = std::env::var_os("HOME")
        .ok_or_else(|| std::io::Error::other("HOME required for test fixtures"))?;
    tempfile::Builder::new()
        .prefix("p")
        .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
        .tempdir_in(Path::new(&parent).canonicalize()?)
}

/// Retries only actual ownership contention, including temporary references from concurrent forks.
fn recover(path: &Path) -> Result<HostState, ProcessStateError> {
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    loop {
        match HostState::recover(path) {
            Err(ProcessStateError::Io(error))
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(/*millis*/ 5));
            }
            result => return result,
        }
    }
}

/// Verifies durable deduplication and monotonic host replacement without changing original intent.
#[test]
fn recovery_keeps_original_creation_intent_and_stable_lock() -> TestResult {
    let directory = directory()?;
    let path = directory.path().join("s");
    let scope = ScopeId::new();
    let mut first = HostState::create(&path)?;
    let binding = first.binding();
    let intent = first.record_scope_intent(scope)?;
    assert_eq!(first.record_scope_intent(scope)?, intent);
    assert_eq!(first.scope_intent(ScopeId::new())?, None);
    let inode = fs::metadata(path.join("host.lock"))?.ino();
    assert!(
        matches!(HostState::recover(&path), Err(ProcessStateError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    drop(first);

    let mut second = recover(&path)?;
    assert_eq!(second.binding().epoch.get(), binding.epoch.get() + 1);
    assert_ne!(second.binding().instance, binding.instance);
    assert_eq!(second.scope_intent(scope)?, Some(intent.clone()));
    assert_eq!(second.record_scope_intent(scope)?, intent);
    assert_eq!(fs::metadata(path.join("host.lock"))?.ino(), inode);
    assert_eq!(fs::read_dir(path.join("scopes"))?.count(), 0);
    // Recording intent alone must not create a guardian directory or launch a Run.
    Ok(())
}

/// Migrates the exact intent-only layout without replacing identities or the stable lock inode.
#[test]
fn version_one_upgrade_preserves_intent_and_ownership() -> TestResult {
    let directory = directory()?;
    let path = directory.path().join("s");
    let scope = ScopeId::new();
    let mut state = HostState::create(&path)?;
    let intent = state.record_scope_intent(scope)?;
    let inode = fs::metadata(path.join("host.lock"))?.ino();
    drop(state);
    let old = rusqlite::Connection::open(path.join("host.sqlite"))?;
    old.execute_batch(
        "DROP TABLE run_observations; DROP TABLE scope_observations; DROP TABLE run_stop_intents; DROP TABLE scope_close_intents; DROP TABLE run_intents; DROP TABLE guardian_launches; PRAGMA user_version=1;",
    )?;
    drop(old);

    let state = recover(&path)?;
    assert_eq!(state.scope_intent(scope)?, Some(intent.clone()));
    assert_eq!(state.guardian_access(scope)?, None);
    assert_eq!(
        state.binding().epoch.get(),
        intent.created_by.epoch.get() + 1
    );
    assert_eq!(fs::metadata(path.join("host.lock"))?.ino(), inode);
    let database = rusqlite::Connection::open(path.join("host.sqlite"))?;
    assert_eq!(
        database.query_row("PRAGMA user_version", [], |row| row
            .get::<_, i64>(/*idx*/ 0))?,
        6
    );
    assert_eq!(fs::read_dir(path.join("scopes"))?.count(), 0);
    Ok(())
}

/// The known token-bearing layout loses only its credential column, never an attempted launch.
#[test]
fn version_two_upgrade_preserves_consumed_launches() -> TestResult {
    let directory = directory()?;
    let path = directory.path().join("s");
    let mut state = HostState::create(&path)?;
    let scope = ScopeId::new();
    let intent = state.record_scope_intent(scope)?;
    let inode = fs::metadata(path.join("host.lock"))?.ino();
    drop(state);
    let old = rusqlite::Connection::open(path.join("host.sqlite"))?;
    old.execute_batch(
        "DROP TABLE run_observations; DROP TABLE scope_observations; DROP TABLE run_stop_intents; DROP TABLE scope_close_intents; DROP TABLE run_intents; DROP TABLE guardian_launches;
CREATE TABLE guardian_launches (
    scope TEXT PRIMARY KEY NOT NULL REFERENCES scope_intents(scope),
    credential BLOB NOT NULL CHECK (length(credential) = 32),
    phase TEXT NOT NULL CHECK (phase = 'launch_unknown')
) STRICT;
PRAGMA user_version=2;",
    )?;
    old.execute(
        "INSERT INTO guardian_launches VALUES (?1, zeroblob(32), 'launch_unknown')",
        [scope.to_string()],
    )?;
    drop(old);
    let legacy_scope = path.join("scopes").join(scope.to_string());
    fs::create_dir(&legacy_scope)?;
    fs::set_permissions(&legacy_scope, fs::Permissions::from_mode(/*mode*/ 0o700))?;
    assert!(matches!(
        recover(&path),
        Err(ProcessStateError::Rejected(
            "legacy guardian scopes require a compatible host version"
        ))
    ));
    let preserved = rusqlite::Connection::open_with_flags(
        path.join("host.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    assert_eq!(preserved.query_row("SELECT (SELECT user_version FROM pragma_user_version), epoch, instance, (SELECT credential FROM guardian_launches) FROM host_binding", [], |row| Ok((row.get::<_, i64>(/*idx*/ 0)?, row.get::<_, i64>(/*idx*/ 1)?, row.get::<_, String>(/*idx*/ 2)?, row.get::<_, Vec<u8>>(/*idx*/ 3)?)))?, (2, intent.created_by.epoch.get() as i64, intent.created_by.instance.to_string(), vec![0; 32]));
    assert_eq!(fs::metadata(path.join("host.lock"))?.ino(), inode);
    drop(preserved);
    // Only remove this empty synthetic test directory; production never erases legacy scopes.
    fs::remove_dir(&legacy_scope)?;
    let state = recover(&path)?;
    assert_eq!(
        state.guardian_access(scope)?,
        Some(ora_process_protocol::GuardianAccess {
            scope_dir: path.join("scopes").join(scope.to_string()),
            intent
        })
    );
    assert_eq!(fs::metadata(path.join("host.lock"))?.ino(), inode);
    drop(state);
    // Reopen also checks that migration produced exactly the new canonical schema.
    assert!(recover(&path)?.guardian_access(scope)?.is_some());
    Ok(())
}

/// Simultaneous creation cannot overwrite a winner's directory or create two host authorities.
#[test]
fn concurrent_creation_has_one_owner() -> TestResult {
    let directory = directory()?;
    let path = directory.path().join("s");
    let barrier = Barrier::new(/*n*/ 2);
    let results = std::thread::scope(|threads| {
        let left = threads.spawn(|| {
            barrier.wait();
            HostState::create(&path)
        });
        let right = threads.spawn(|| {
            barrier.wait();
            HostState::create(&path)
        });
        Ok::<_, Box<dyn std::error::Error>>([
            left.join().map_err(|_| "left creator panicked")?,
            right.join().map_err(|_| "right creator panicked")?,
        ])
    })?;
    let mut outcomes = results.each_ref().map(Result::is_ok);
    outcomes.sort();
    assert_eq!(outcomes, [false, true]);
    for result in results {
        match result {
            Ok(mut state) => {
                let scope = ScopeId::new();
                let intent = state.record_scope_intent(scope)?;
                assert_eq!(state.scope_intent(scope)?, Some(intent));
            }
            Err(ProcessStateError::Io(error)) => {
                assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists)
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

/// Refuses ambiguous initialization and leaves both absent recovery targets and old data untouched.
#[test]
fn creation_and_recovery_never_reinitialize_existing_paths() -> TestResult {
    let directory = directory()?;
    let absent = directory.path().join("absent");
    assert!(HostState::recover(&absent).is_err());
    assert!(!absent.exists());
    let occupied = directory.path().join("s");
    fs::create_dir(&occupied)?;
    assert!(HostState::create(&occupied).is_err());
    fs::write(occupied.join("host.sqlite"), b"unrelated user data")?;
    assert!(HostState::create(&occupied).is_err());
    assert!(HostState::recover(&occupied).is_err());
    assert_eq!(
        fs::read(occupied.join("host.sqlite"))?,
        b"unrelated user data"
    );
    assert!(!occupied.join("host.lock").exists());
    assert!(HostState::create(Path::new("relative-state")).is_err());
    let too_long = directory.path().join("x".repeat(90));
    assert!(HostState::create(&too_long).is_err());
    assert!(!too_long.exists());
    Ok(())
}

/// Rejects private-path violations before journal mutation; no chmod, unlink or reset is attempted.
#[test]
fn recovery_rejects_links_permissions_and_unknown_files() -> TestResult {
    let directory = directory()?;
    let path = directory.path().join("s");
    drop(HostState::create(&path)?);
    let original = fs::read(path.join("host.sqlite"))?;
    let alias = directory.path().join("a");
    symlink(&path, &alias)?;
    assert!(HostState::recover(&alias).is_err());

    let database = path.join("host.sqlite");
    let saved = directory.path().join("saved");
    fs::rename(&database, &saved)?;
    symlink(&saved, &database)?;
    assert!(HostState::recover(&path).is_err());
    assert_eq!(fs::read(&saved)?, original);
    fs::remove_file(&database)?;
    fs::rename(&saved, &database)?;

    fs::hard_link(&database, &saved)?;
    assert!(HostState::recover(&path).is_err());
    fs::remove_file(&saved)?;
    fs::set_permissions(&database, fs::Permissions::from_mode(/*mode*/ 0o644))?;
    assert!(HostState::recover(&path).is_err());
    assert_eq!(fs::metadata(&database)?.mode() & 0o777, 0o644);
    fs::set_permissions(&database, fs::Permissions::from_mode(/*mode*/ 0o600))?;

    fs::write(path.join("unknown"), b"preserve")?;
    assert!(HostState::recover(&path).is_err());
    assert_eq!(
        (fs::read(&database)?, fs::read(path.join("unknown"))?),
        (original, b"preserve".to_vec())
    );
    fs::remove_file(path.join("unknown"))?;
    drop(recover(&path)?);
    Ok(())
}

/// Missing authority files are not evidence that a directory can be initialized again.
#[test]
fn missing_lock_or_journal_is_not_recreated() -> TestResult {
    let directory = directory()?;
    for missing in ["host.lock", "host.sqlite", "scopes"] {
        let path = directory.path().join(missing);
        drop(HostState::create(&path)?);
        let target = path.join(missing);
        let saved = directory.path().join("saved");
        fs::rename(&target, &saved)?;
        assert!(HostState::recover(&path).is_err());
        assert!(!target.exists());
        fs::rename(&saved, &target)?;
        drop(recover(&path)?);
    }
    Ok(())
}

/// An existing scope without host responsibility cannot be accepted as an empty recovered host.
#[test]
fn unknown_scope_directories_block_recovery_without_changing_binding() -> TestResult {
    let directory = directory()?;
    let path = directory.path().join("s");
    let binding = HostState::create(&path)?.binding();
    let orphan = path.join("scopes").join(ScopeId::new().to_string());
    fs::create_dir(&orphan)?;
    fs::set_permissions(&orphan, fs::Permissions::from_mode(/*mode*/ 0o700))?;
    assert!(matches!(
        HostState::recover(&path),
        Err(ProcessStateError::Rejected(_))
    ));
    assert!(orphan.is_dir());
    fs::rename(&orphan, directory.path().join("preserved"))?;
    assert_eq!(
        recover(&path)?.binding().epoch.get(),
        binding.epoch.get() + 1
    );
    Ok(())
}

/// Unknown versions and unexpected SQL objects fail read-only preflight without changing epochs.
#[test]
fn incompatible_or_corrupt_journals_are_preserved() -> TestResult {
    let directory = directory()?;
    for statement in [
        "PRAGMA user_version=99",
        "PRAGMA application_id=0",
        "CREATE TABLE foreign_data (value TEXT)",
        "UPDATE host_binding SET epoch=9223372036854775807",
        "UPDATE host_binding SET instance='not-an-identity'",
        "DELETE FROM host_binding",
        "UPDATE scope_intents SET guardian='not-an-identity'",
        "UPDATE scope_intents SET host_epoch=2",
    ] {
        let child = tempfile::Builder::new()
            .prefix("j")
            .permissions(fs::Permissions::from_mode(/*mode*/ 0o700))
            .tempdir_in(directory.path())?;
        let path = child.path().join("s");
        let mut state = HostState::create(&path)?;
        state.record_scope_intent(ScopeId::new())?;
        drop(state);
        let database = path.join("host.sqlite");
        let connection = rusqlite::Connection::open(&database)?;
        connection.execute_batch(statement)?;
        drop(connection);
        let original = fs::read(&database)?;
        assert!(HostState::recover(&path).is_err(), "accepted {statement}");
        assert_eq!(fs::read(&database)?, original);
    }
    Ok(())
}

/// Failed admission retains unrelated intent and can proceed after an external conflict is repaired.
#[test]
fn scope_path_conflict_does_not_accept_or_erase_responsibility() -> TestResult {
    let directory = directory()?;
    let path = directory.path().join("s");
    let mut state = HostState::create(&path)?;
    let original = state.record_scope_intent(ScopeId::new())?;
    let scope = ScopeId::new();
    let conflicting = path.join("scopes").join(scope.to_string());
    fs::write(&conflicting, b"unknown scope")?;
    assert!(state.record_scope_intent(scope).is_err());
    assert_eq!(state.scope_intent(scope)?, None);
    assert_eq!(state.scope_intent(original.scope)?, Some(original));
    assert_eq!(fs::read(&conflicting)?, b"unknown scope");
    fs::rename(&conflicting, directory.path().join("preserved"))?;
    assert_eq!(state.record_scope_intent(scope)?.scope, scope);
    Ok(())
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    /// Keeps failed crash assertions from leaving fixture processes alive.
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// External SIGKILL loses the caller's in-memory result but not the original committed intent.
#[test]
fn committed_intent_survives_owner_kill_without_becoming_a_new_attempt() -> TestResult {
    let directory = directory()?;
    let path = directory.path().join("s");
    let marker = directory.path().join("ready");
    let scope = ScopeId::new();
    let mut child = ChildGuard(
        Command::new(std::env::current_exe()?)
            .args(["--exact", "host_state_fixture", "--nocapture"])
            .env("ORA_HOST_STATE_FIXTURE", &path)
            .env("ORA_HOST_STATE_MARKER", &marker)
            .env("ORA_HOST_STATE_SCOPE", scope.to_string())
            .env("HOME", directory.path().join("unused-home"))
            .current_dir(directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(/*secs*/ 10);
    let fields = loop {
        if let Ok(marker) = fs::read_to_string(&marker) {
            let fields = marker.split(':').map(str::to_owned).collect::<Vec<_>>();
            if fields.len() == 4 {
                break fields;
            }
        }
        assert!(
            child.0.try_wait()?.is_none(),
            "fixture exited before committing"
        );
        assert!(Instant::now() < deadline, "fixture did not commit");
        std::thread::sleep(Duration::from_millis(/*millis*/ 5));
    };
    let original = ScopeCreationIntent {
        scope: fields[0].parse()?,
        guardian: fields[1].parse()?,
        created_by: HostBinding {
            epoch: fields[2].parse()?,
            instance: fields[3].parse()?,
        },
    };
    assert!(
        matches!(HostState::recover(&path), Err(ProcessStateError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    child.0.kill()?;
    child.0.wait()?;
    let mut recovered = recover(&path)?;
    assert_eq!(recovered.record_scope_intent(scope)?, original);
    assert_eq!(
        recovered.binding().epoch.get(),
        original.created_by.epoch.get() + 1
    );
    assert!(!directory.path().join("unused-home").exists());
    Ok(())
}

/// Acts only as a journal owner; this fixture is not a host app, guardian or launch RPC.
#[test]
fn host_state_fixture() -> TestResult {
    let Some(path) = std::env::var_os("ORA_HOST_STATE_FIXTURE") else {
        return Ok(());
    };
    let marker = PathBuf::from(std::env::var_os("ORA_HOST_STATE_MARKER").ok_or("missing marker")?);
    let scope = std::env::var("ORA_HOST_STATE_SCOPE")?.parse()?;
    let mut state = HostState::create(Path::new(&path))?;
    let intent = state.record_scope_intent(scope)?;
    fs::write(
        marker.with_extension("pending"),
        format!(
            "{}:{}:{}:{}",
            intent.scope, intent.guardian, intent.created_by.epoch, intent.created_by.instance
        ),
    )?;
    fs::rename(marker.with_extension("pending"), marker)?;
    // Bounded even if the external driver disappears; the driver kills it before normal Drop.
    std::thread::sleep(Duration::from_secs(/*secs*/ 20));
    Ok(())
}
