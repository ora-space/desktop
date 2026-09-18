use std::fs::{self, File, OpenOptions};
use std::os::fd::OwnedFd;
use std::os::unix::{
    fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    net::UnixStream,
};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use ora_process_protocol::{
    GUARDIAN_WIRE_VERSION, GuardianAccess, GuardianBootstrap, ScopeId, encode_guardian_frame,
};
use ora_utils::{
    fs::LinuxFileLock,
    path::{TrustedPathKind, open_trusted_path},
    process::configure_linux_detached_child,
};
use rusqlite::{OptionalExtension, params};
use tokio::io::AsyncWriteExt;

use super::{HostState, ProcessStateError};

/// Owns only bootstrap delivery; dropping it never grants another launch attempt.
pub(crate) struct GuardianLaunch {
    access: GuardianAccess,
    parent: tokio::net::UnixStream,
    bootstrap: Vec<u8>,
}

impl GuardianLaunch {
    /// Delivers outside host ownership's mutex so a slow guardian cannot stall other Scopes.
    pub(crate) async fn deliver(mut self) -> Result<GuardianAccess, ProcessStateError> {
        tokio::time::timeout(
            Duration::from_secs(/*secs*/ 5),
            self.parent.write_all(&self.bootstrap),
        )
        .await
        .map_err(|_| {
            std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "guardian bootstrap delivery timed out",
            )
        })??;
        Ok(self.access)
    }
}

impl HostState {
    /// Retrieves the original identity for discovery, never permission to launch again.
    pub fn guardian_access(
        &self,
        scope: ScopeId,
    ) -> Result<Option<GuardianAccess>, ProcessStateError> {
        let attempted = self
            .connection
            .query_row(
                "SELECT phase FROM guardian_launches WHERE scope=?1",
                [scope.to_string()],
                |row| row.get::<_, String>(/*idx*/ 0),
            )
            .optional()?;
        let Some(_phase) = attempted else {
            return Ok(None);
        };
        let intent = self
            .scope_intent(scope)?
            .ok_or(ProcessStateError::Rejected("launch has no scope intent"))?;
        Ok(Some(GuardianAccess {
            scope_dir: self.layout.scope_path(scope),
            intent,
        }))
    }

    /// Records a one-shot launch boundary before creating the independent guardian.
    ///
    /// Every failure after the transaction, including cancellation, remains launch-unknown. Reuse
    /// guardian_access for discovery; even proven exec errors do not authorize a second attempt.
    pub async fn start_guardian(
        &mut self,
        scope: ScopeId,
        executable: &Path,
    ) -> Result<GuardianAccess, ProcessStateError> {
        self.begin_guardian(scope, executable)?.deliver().await
    }

    /// Commits the launch boundary and spawns without retaining host ownership during socket I/O.
    pub(crate) fn begin_guardian(
        &mut self,
        scope: ScopeId,
        executable: &Path,
    ) -> Result<GuardianLaunch, ProcessStateError> {
        if self.scope_close_requested(scope)? {
            return Err(ProcessStateError::Rejected("host Scope is closing"));
        }
        if self.guardian_access(scope)?.is_some() {
            return Err(ProcessStateError::Rejected(
                "guardian launch already attempted; discover original instance",
            ));
        }
        let intent = self
            .scope_intent(scope)?
            .ok_or(ProcessStateError::Rejected(
                "scope intent must be recorded before guardian launch",
            ))?;
        self.layout.reject_existing_scope(scope)?;
        // SAFETY: geteuid queries the current OS identity without mutation.
        let owner = unsafe { libc::geteuid() };
        let program = open_trusted_path(executable, owner, TrustedPathKind::File)?;
        if program.metadata()?.mode() & 0o6000 != 0 {
            return Err(ProcessStateError::Rejected(
                "guardian executable must not be set-id",
            ));
        }
        let access = GuardianAccess {
            scope_dir: self.layout.scope_path(scope),
            intent,
        };
        let bootstrap = encode_guardian_frame(&GuardianBootstrap {
            version: GUARDIAN_WIRE_VERSION,
            access: access.clone(),
        })?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO guardian_launches VALUES (?1, 'launch_unknown')",
            params![scope.to_string()],
        )?;
        transaction.commit()?;
        self.layout.sync()?;

        fs::DirBuilder::new()
            .mode(/*mode*/ 0o700)
            .create(&access.scope_dir)?;
        File::open(
            access
                .scope_dir
                .parent()
                .ok_or(ProcessStateError::Rejected("missing scopes parent"))?,
        )?
        .sync_all()?;
        let lock_file = OpenOptions::new()
            .read(/*read*/ true)
            .write(/*write*/ true)
            .create_new(/*create_new*/ true)
            .mode(/*mode*/ 0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(access.scope_dir.join("guardian.lock"))?;
        lock_file.sync_all()?;
        let lock = LinuxFileLock::try_acquire(lock_file)?;
        File::open(&access.scope_dir)?.sync_all()?;
        let (parent, child) = UnixStream::pair()?;
        parent.set_nonblocking(/*nonblocking*/ true)?;
        let parent = tokio::net::UnixStream::from_std(parent)?;
        let mut command = Command::new(executable);
        command
            .arg("--bootstrap")
            .env_clear()
            .current_dir(&access.scope_dir)
            .stdin(Stdio::from(lock.into_file()))
            .stdout(Stdio::from(OwnedFd::from(child)))
            .stderr(Stdio::null());
        configure_linux_detached_child(&mut command);
        // Reserve the reaper before exec so thread exhaustion cannot leave an unowned live child.
        // The waiter owns reaping only, never a Drop-kill policy. Host death re-parents the guardian.
        let (sender, receiver) = std::sync::mpsc::channel::<std::process::Child>();
        std::thread::Builder::new()
            .name("guardian-reap".into())
            .spawn(move || {
                if let Ok(mut child) = receiver.recv() {
                    let _ = child.wait();
                }
            })?;
        let child = command.spawn()?;
        sender
            .send(child)
            .map_err(|_| std::io::Error::other("guardian reaper unexpectedly unavailable"))?;
        // Ready is deliberately queried independently. Losing this result cannot trigger another exec.
        Ok(GuardianLaunch {
            access,
            parent,
            bootstrap,
        })
    }
}
