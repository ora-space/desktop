//! Durable host scope/Run intent and one-shot guardian bootstrap, separate from guardian execution.

mod control;
mod journal;
pub(crate) mod launch;
mod layout;
mod observations;
mod runs;

use std::num::NonZeroU64;
use std::path::Path;

use crate::{ProcessStateError, state_journal};
use ora_process_protocol::{HostBinding, HostInstanceId, ScopeCreationIntent, ScopeId};
use ora_utils::fs::LinuxFileLock;
use rusqlite::Connection;

/// Holds host ownership for its journal lifetime and records each guardian launch before exec.
///
/// The injected directory belongs exclusively to this runtime. Creation and recovery are separate:
/// recovery never creates missing files, resets a journal, or authorizes another guardian launch.
pub struct HostState {
    // Field drop order closes SQLite before releasing filesystem ownership.
    connection: Connection,
    layout: layout::HostLayout,
    binding: HostBinding,
    _lock: LinuxFileLock,
}

impl HostState {
    /// Publishes host-owned endpoints only while this journal owns the stable host lock.
    pub(crate) async fn bind_endpoints(
        &self,
    ) -> Result<(tokio::net::UnixListener, tokio::net::UnixListener), ProcessStateError> {
        Ok((
            self.layout.bind_endpoint("host.sock").await?,
            self.layout.bind_endpoint("host-io.sock").await?,
        ))
    }
    /// Initializes a previously absent dedicated directory; even an existing empty directory fails.
    pub fn create(state_dir: &Path) -> Result<Self, ProcessStateError> {
        state_journal::check_engine()?;
        let layout = layout::HostLayout::create(state_dir)?;
        let lock = LinuxFileLock::try_acquire(layout.create_file("host.lock")?)?;
        layout.create_file("host.sqlite")?;
        let mut connection = state_journal::open_writable(&layout.database_path())?;
        let binding = HostBinding {
            epoch: NonZeroU64::MIN,
            instance: HostInstanceId::new(),
        };
        journal::initialize(&mut connection, binding)?;
        layout.sync()?;
        Ok(Self {
            connection,
            layout,
            binding,
            _lock: lock,
        })
    }

    /// Validates an existing journal under its original lock, then durably advances host identity.
    pub fn recover(state_dir: &Path) -> Result<Self, ProcessStateError> {
        state_journal::check_engine()?;
        let layout = layout::HostLayout::open(state_dir)?;
        let lock = LinuxFileLock::try_acquire(layout.open_file("host.lock")?)?;
        let scopes = layout.validate_entries()?;
        // Read-only compatibility inspection precedes any journal configuration or authority write.
        let (previous, version) = journal::inspect(&layout.database_path(), &scopes)?;
        // Existing v2 scopes may still need their original tokens. Preserve that entire host
        // layout for a compatible binary rather than destroying discovery during migration.
        if version == 2 && !scopes.is_empty() {
            return Err(ProcessStateError::Rejected(
                "legacy guardian scopes require a compatible host version",
            ));
        }
        let epoch = previous
            .epoch
            .get()
            .checked_add(1)
            .filter(|epoch| *epoch <= i64::MAX as u64)
            .and_then(NonZeroU64::new)
            .ok_or(ProcessStateError::Rejected("host epoch exhausted"))?;
        let binding = HostBinding {
            epoch,
            instance: HostInstanceId::new(),
        };
        let mut connection = state_journal::open_writable(&layout.database_path())?;
        journal::advance_binding(&mut connection, binding, version)?;
        layout.sync()?;
        Ok(Self {
            connection,
            layout,
            binding,
            _lock: lock,
        })
    }

    /// Returns this durably committed incarnation; it is not Controller or Scope authorization.
    pub fn binding(&self) -> HostBinding {
        self.binding
    }

    /// Persists one original creation intent, or returns that same intent on duplicate submission.
    ///
    /// No scope directory, guardian journal or process is created. A returned record is never a
    /// spawn ticket: a later bootstrap must durably record its launch boundary before exec.
    pub fn record_scope_intent(
        &mut self,
        scope: ScopeId,
    ) -> Result<ScopeCreationIntent, ProcessStateError> {
        if let Some(intent) = journal::find_intent(&self.connection, scope)? {
            // A prior call may have committed but failed its directory sync. Replays must cross
            // that remaining durability boundary too, rather than acknowledging it from cache.
            self.layout.sync()?;
            return Ok(intent);
        }
        self.layout.reject_existing_scope(scope)?;
        let intent = ScopeCreationIntent {
            scope,
            guardian: ora_process_protocol::GuardianInstanceId::new(),
            created_by: self.binding,
        };
        journal::insert_intent(&mut self.connection, &intent)?;
        self.layout.sync()?;
        Ok(intent)
    }

    /// Queries persisted responsibility without inferring that a guardian exists or is Ready.
    pub fn scope_intent(
        &self,
        scope: ScopeId,
    ) -> Result<Option<ScopeCreationIntent>, ProcessStateError> {
        journal::find_intent(&self.connection, scope)
    }
}
