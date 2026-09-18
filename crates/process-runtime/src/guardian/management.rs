use ora_process_protocol::{
    GuardianChannel, GuardianHostSession, GuardianManagementOperation,
    GuardianManagementRejection as Rejection, GuardianManagementReply, HostBinding,
    ScopeCreationIntent,
};
use rusqlite::{Connection, params};

#[cfg(test)]
#[path = "management_tests.rs"]
mod tests;

/// Creates the initial session in the same transaction as guardian initialization.
pub(super) fn initialize(
    transaction: &rusqlite::Transaction<'_>,
    host: HostBinding,
) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "CREATE TABLE guardian_host_session (
        singleton INTEGER PRIMARY KEY CHECK (singleton=1),
        host_epoch INTEGER NOT NULL CHECK (host_epoch>0), host_instance TEXT NOT NULL
    ) STRICT;",
    )?;
    transaction.execute(
        "INSERT INTO guardian_host_session VALUES (1, ?1, ?2)",
        params![
            i64::try_from(host.epoch.get()).map_err(|_| rusqlite::Error::InvalidQuery)?,
            host.instance.to_string()
        ],
    )?;
    Ok(())
}

/// Owns the persisted binding and execution gate; callers hold one mutex across check and effect.
pub(super) struct Management {
    // Drop tracked workloads before the database and stable scope lock.
    runs: super::runs::Runs,
    connection: Connection,
    // No cached binding: even a commit whose acknowledgement failed must be read back from SQLite.
    intent: ScopeCreationIntent,
    // Keep the stable lock until the last executing worker closes SQLite, including cancellation.
    _lock: ora_utils::fs::LinuxFileLock,
}

impl Management {
    pub(super) fn new(
        connection: Connection,
        intent: ScopeCreationIntent,
        lock: ora_utils::fs::LinuxFileLock,
    ) -> Result<Self, crate::ProcessStateError> {
        Ok(Self {
            runs: super::runs::Runs::new()?,
            connection,
            intent,
            _lock: lock,
        })
    }

    /// Advances accepted responsibilities independently of client connection lifetime.
    pub(super) fn reconcile(&mut self) {
        // The next request reports storage errors; the runtime still advances existing stop plans.
        let _ = self.runs.reconcile(&mut self.connection);
    }

    /// Uses the management owner for stale-host checks immediately before executing a Run operation.
    pub(super) fn execute_run(
        &mut self,
        channel: GuardianChannel,
        request: ora_process_protocol::GuardianRunRequest,
    ) -> ora_process_protocol::GuardianRunReply {
        use ora_process_protocol::{GuardianRunRejection, GuardianRunResult};
        let result = if channel != request.operation.channel() {
            GuardianRunResult::Rejected(GuardianRunRejection::Management(Rejection::WrongChannel))
        } else {
            match self.apply(
                channel,
                GuardianManagementOperation::Inspect {
                    session: request.session.clone(),
                },
            ) {
                Ok(_) => self.runs.execute(&mut self.connection, request.operation),
                Err(reason) => {
                    GuardianRunResult::Rejected(GuardianRunRejection::Management(reason))
                }
            }
        };
        ora_process_protocol::GuardianRunReply {
            intent: self.intent.clone(),
            session: request.session,
            result,
        }
    }

    /// Executes an already decoded request against current authority, never an ingress-time snapshot.
    pub(super) fn execute(
        &mut self,
        channel: GuardianChannel,
        operation: GuardianManagementOperation,
    ) -> GuardianManagementReply {
        match self.apply(channel, operation) {
            Ok(reply) => reply,
            Err(reason) => GuardianManagementReply::Rejected(reason),
        }
    }

    /// Serializes durable takeover and session inspection at their actual execution point.
    fn apply(
        &mut self,
        channel: GuardianChannel,
        operation: GuardianManagementOperation,
    ) -> Result<GuardianManagementReply, Rejection> {
        // A failed rollback must not expose an uncommitted binding through this connection.
        if !self.connection.is_autocommit() {
            return Err(Rejection::StorageUnavailable);
        }
        let current = self
            .connection
            .query_row(
                "SELECT host_epoch, host_instance FROM guardian_host_session WHERE singleton=1",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(/*idx*/ 0)?,
                        row.get::<_, String>(/*idx*/ 1)?,
                    ))
                },
            )
            .map_err(|_| Rejection::StorageUnavailable)?;
        let current = GuardianHostSession {
            host: HostBinding {
                epoch: u64::try_from(current.0)
                    .ok()
                    .and_then(std::num::NonZeroU64::new)
                    .ok_or(Rejection::StorageUnavailable)?,
                instance: current
                    .1
                    .parse()
                    .map_err(|_| Rejection::StorageUnavailable)?,
            },
        };
        match operation {
            GuardianManagementOperation::Bind { host } => {
                if channel != GuardianChannel::Control {
                    return Err(Rejection::WrongChannel);
                }
                let epoch = i64::try_from(host.epoch.get()).map_err(|_| Rejection::InvalidEpoch)?;
                if host.epoch < current.host.epoch {
                    return Err(Rejection::StaleHost);
                }
                if host.epoch == current.host.epoch && host.instance != current.host.instance {
                    return Err(Rejection::ConflictingHost);
                }
                let session = if host == current.host {
                    current
                } else {
                    let session = GuardianHostSession { host };
                    let transaction = self
                        .connection
                        .transaction()
                        .map_err(|_| Rejection::StorageUnavailable)?;
                    let changed = transaction.execute(
                        "UPDATE guardian_host_session SET host_epoch=?1, host_instance=?2 WHERE singleton=1",
                        params![epoch, host.instance.to_string()],
                    ).map_err(|_| Rejection::StorageUnavailable)?;
                    if changed != 1 {
                        return Err(Rejection::StorageUnavailable);
                    }
                    transaction
                        .commit()
                        .map_err(|_| Rejection::StorageUnavailable)?;
                    session
                };
                Ok(GuardianManagementReply::Bound {
                    intent: self.intent.clone(),
                    session,
                })
            }
            GuardianManagementOperation::Inspect { session } => {
                if session != current {
                    return Err(Rejection::StaleSession);
                }
                Ok(GuardianManagementReply::Current {
                    intent: self.intent.clone(),
                    session,
                    channel,
                })
            }
        }
    }
}
