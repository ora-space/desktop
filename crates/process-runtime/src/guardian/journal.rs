use crate::{ProcessStateError, state_journal};
use ora_process_protocol::GuardianAccess;
use ora_utils::{
    fs::{LinuxFileLock, LinuxFilesystem},
    path::{TrustedPathKind, open_private_path},
};
use rusqlite::{Connection, params};
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};

/// Initializes only a new original guardian journal, after matching inherited lock and scope path.
pub(super) fn initialize(
    access: &GuardianAccess,
    lock: &LinuxFileLock,
    owner: u32,
) -> Result<Connection, ProcessStateError> {
    let directory = open_private_path(&access.scope_dir, owner, TrustedPathKind::Directory)?;
    if matches!(
        LinuxFilesystem::for_file(&directory)?,
        LinuxFilesystem::Other
    ) {
        return Err(ProcessStateError::Rejected(
            "unsupported guardian filesystem",
        ));
    }
    if access.scope_dir.file_name() != Some(std::ffi::OsStr::new(&access.intent.scope.to_string()))
    {
        return Err(ProcessStateError::Rejected(
            "scope directory identity mismatch",
        ));
    }
    let original = open_private_path(
        &access.scope_dir.join("guardian.lock"),
        owner,
        TrustedPathKind::File,
    )?
    .metadata()?;
    let inherited = lock.try_clone()?.into_file().metadata()?;
    if (original.dev(), original.ino()) != (inherited.dev(), inherited.ino()) {
        return Err(ProcessStateError::Rejected(
            "inherited scope lock identity mismatch",
        ));
    }
    for entry in fs::read_dir(&access.scope_dir)? {
        if entry?.file_name() != "guardian.lock" {
            return Err(ProcessStateError::Rejected(
                "scope already initialized or contains unknown files",
            ));
        }
    }
    let path = access.scope_dir.join("guardian.sqlite");
    let file = OpenOptions::new()
        .write(/*write*/ true)
        .create_new(/*create_new*/ true)
        .mode(/*mode*/ 0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)?;
    file.sync_all()?;
    let mut connection = state_journal::open_writable(&path)?;
    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "PRAGMA application_id=1330790727; PRAGMA user_version=4;
        CREATE TABLE guardian_bootstrap (
            singleton INTEGER PRIMARY KEY CHECK (singleton=1), scope TEXT NOT NULL,
            guardian TEXT NOT NULL, host_epoch INTEGER NOT NULL CHECK (host_epoch>0),
            host_instance TEXT NOT NULL,
            phase TEXT NOT NULL CHECK (phase='initialized')
        ) STRICT;",
    )?;
    let epoch = i64::try_from(access.intent.created_by.epoch.get())
        .map_err(|_| ProcessStateError::Rejected("host epoch outside journal range"))?;
    transaction.execute(
        "INSERT INTO guardian_bootstrap VALUES (1, ?1, ?2, ?3, ?4, 'initialized')",
        params![
            access.intent.scope.to_string(),
            access.intent.guardian.to_string(),
            epoch,
            access.intent.created_by.instance.to_string()
        ],
    )?;
    super::management::initialize(&transaction, access.intent.created_by)?;
    super::runs::initialize(&transaction)?;
    transaction.commit()?;
    fs::DirBuilder::new()
        .mode(/*mode*/ 0o700)
        .create(access.scope_dir.join("output"))?;
    File::open(access.scope_dir.join("output"))?.sync_all()?;
    directory.sync_all()?;
    Ok(connection)
}
