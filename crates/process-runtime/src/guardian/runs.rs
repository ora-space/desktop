use std::time::Instant;

use ora_process_protocol::{
    CleanupState, ContainmentRequest, DirectProcessState, GUARDIAN_CAPTURE_LIMIT,
    GUARDIAN_OUTPUT_CHUNK_LIMIT, GuardianHostDisconnect, GuardianRunOperation,
    GuardianRunRejection as Rejection, GuardianRunResult as Reply, LaunchFact, OutputPolicy, RunId,
    RunSnapshot, RunSpec, ScopeState, StopRequest, decode_guardian_payload, encode_guardian_frame,
};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{LinuxBestEffort, ProcessStateError, ScopeRuntime};

/// Commits an empty run ledger as part of original guardian initialization, never recovery.
pub(super) fn initialize(transaction: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "CREATE TABLE guardian_run_scope (
            singleton INTEGER PRIMARY KEY CHECK (singleton=1), state BLOB NOT NULL
        ) STRICT;
        CREATE TABLE guardian_runs (
            run TEXT PRIMARY KEY NOT NULL, spec BLOB NOT NULL, snapshot BLOB NOT NULL,
            host_disconnect TEXT NOT NULL CHECK (host_disconnect='keep_running'),
            stop_requested INTEGER NOT NULL CHECK (stop_requested IN (0,1))
        ) STRICT;",
    )?;
    let state =
        encode_guardian_frame(&ScopeState::Open).map_err(|_| rusqlite::Error::InvalidQuery)?;
    transaction.execute(
        "INSERT INTO guardian_run_scope VALUES (1, ?1)",
        [&state[4..]],
    )?;
    Ok(())
}

/// Only this original guardian can connect durable acceptance with live process identities.
/// Stored attempts without live handles remain uncertain; journal data never authorizes respawn.
pub(super) struct Runs {
    runtime: ScopeRuntime<LinuxBestEffort>,
    live: Vec<RunId>,
}

impl Runs {
    pub(super) fn new() -> Result<Self, ProcessStateError> {
        let runtime = ScopeRuntime::new(
            ContainmentRequest::BestEffort,
            LinuxBestEffort::with_bounded_output()?,
        )
        .map_err(|_| ProcessStateError::Rejected("rootless runtime unavailable"))?;
        Ok(Self {
            runtime,
            live: Vec::new(),
        })
    }

    /// Runs under the same execution mutex as host takeover; no await separates checks from effects.
    pub(super) fn execute(
        &mut self,
        connection: &mut Connection,
        operation: GuardianRunOperation,
    ) -> Reply {
        match self.apply(connection, operation) {
            Ok(reply) => reply,
            Err(reason) => Reply::Rejected(reason),
        }
    }

    /// Persists intent before each side effect and exposes only read-back journal snapshots.
    fn apply(
        &mut self,
        connection: &mut Connection,
        operation: GuardianRunOperation,
    ) -> Result<Reply, Rejection> {
        if !connection.is_autocommit() {
            return Err(Rejection::StorageUnavailable);
        }
        match operation {
            GuardianRunOperation::Start {
                run,
                spec,
                host_disconnect: GuardianHostDisconnect::KeepRunning,
            } => {
                self.start(connection, run, spec)?;
                self.reconcile(connection)?;
                Ok(Reply::Run(snapshot(connection, run)?))
            }
            GuardianRunOperation::Query { run } => {
                self.reconcile(connection)?;
                Ok(Reply::Run(snapshot(connection, run)?))
            }
            GuardianRunOperation::Stop { run } => {
                snapshot(connection, run)?;
                connection
                    .execute(
                        "UPDATE guardian_runs SET stop_requested=1 WHERE run=?1",
                        [run.to_string()],
                    )
                    .map_err(|_| Rejection::StorageUnavailable)?;
                self.reconcile(connection)?;
                Ok(Reply::Run(snapshot(connection, run)?))
            }
            GuardianRunOperation::Close => {
                if scope_state(connection)? == ScopeState::Open {
                    let state = encode_guardian_frame(&ScopeState::Closing)
                        .map_err(|_| Rejection::StorageUnavailable)?;
                    let transaction = connection
                        .transaction()
                        .map_err(|_| Rejection::StorageUnavailable)?;
                    transaction
                        .execute(
                            "UPDATE guardian_run_scope SET state=?1 WHERE singleton=1",
                            [&state[4..]],
                        )
                        .map_err(|_| Rejection::StorageUnavailable)?;
                    transaction
                        .execute("UPDATE guardian_runs SET stop_requested=1", [])
                        .map_err(|_| Rejection::StorageUnavailable)?;
                    transaction
                        .commit()
                        .map_err(|_| Rejection::StorageUnavailable)?;
                }
                self.reconcile(connection)?;
                Ok(Reply::Scope(scope_state(connection)?))
            }
            GuardianRunOperation::Scope => {
                self.reconcile(connection)?;
                Ok(Reply::Scope(scope_state(connection)?))
            }
            GuardianRunOperation::Output {
                run,
                stream,
                offset,
                max_bytes,
            } => {
                if max_bytes > GUARDIAN_OUTPUT_CHUNK_LIMIT {
                    return Err(Rejection::LimitExceeded);
                }
                snapshot(connection, run)?;
                let output = self
                    .runtime
                    .read_output(run, stream, offset, max_bytes)
                    .map_err(|_| Rejection::OutputUnavailable)?;
                Ok(Reply::Output {
                    run,
                    stream,
                    offset,
                    output,
                })
            }
        }
    }

    /// Replays an accepted identity without exec, including an uncertain post-commit outcome.
    fn start(
        &mut self,
        connection: &mut Connection,
        run: RunId,
        spec: RunSpec,
    ) -> Result<(), Rejection> {
        let existing: Option<Vec<u8>> = connection
            .query_row(
                "SELECT spec FROM guardian_runs WHERE run=?1",
                [run.to_string()],
                |row| row.get(/*idx*/ 0),
            )
            .optional()
            .map_err(|_| Rejection::StorageUnavailable)?;
        if let Some(existing) = existing {
            let original: RunSpec =
                decode_guardian_payload(&existing).map_err(|_| Rejection::StorageUnavailable)?;
            return if original == spec {
                Ok(())
            } else {
                Err(Rejection::ConflictingRun)
            };
        }
        if scope_state(connection)? != ScopeState::Open {
            return Err(Rejection::ScopeClosed);
        }
        let count: i64 = connection
            .query_row("SELECT count(*) FROM guardian_runs", [], |row| {
                row.get(/*idx*/ 0)
            })
            .map_err(|_| Rejection::StorageUnavailable)?;
        if count >= ora_process_protocol::GUARDIAN_RUN_LIMIT as i64
            || matches!(spec.output, OutputPolicy::Capture { stdout_limit, stderr_limit } if stdout_limit > GUARDIAN_CAPTURE_LIMIT || stderr_limit > GUARDIAN_CAPTURE_LIMIT)
        {
            return Err(Rejection::LimitExceeded);
        }
        let encoded_spec = encode_guardian_frame(&spec).map_err(|_| Rejection::LimitExceeded)?;
        let unknown = RunSnapshot {
            id: run,
            launch: LaunchFact::Unknown("accepted; execution not yet observed".into()),
            direct: DirectProcessState::Unknown,
            cleanup: CleanupState::Pending,
        };
        let encoded_snapshot =
            encode_guardian_frame(&unknown).map_err(|_| Rejection::StorageUnavailable)?;
        // An error from commit is ambiguous. A retry reads this row before considering any exec.
        let transaction = connection
            .transaction()
            .map_err(|_| Rejection::StorageUnavailable)?;
        transaction
            .execute(
                "INSERT INTO guardian_runs VALUES (?1, ?2, ?3, 'keep_running', 0)",
                params![run.to_string(), &encoded_spec[4..], &encoded_snapshot[4..]],
            )
            .map_err(|_| Rejection::StorageUnavailable)?;
        transaction
            .commit()
            .map_err(|_| Rejection::StorageUnavailable)?;
        self.live.push(run);
        self.runtime
            .start(run, spec)
            .map_err(|_| Rejection::StorageUnavailable)?;
        Ok(())
    }

    /// Continues cleanup without callers; journal failure never cancels already accepted stop plans.
    pub(super) fn reconcile(&mut self, connection: &mut Connection) -> Result<(), Rejection> {
        let intents = self.restore_stop_intents(connection);
        self.runtime.reconcile(Instant::now());
        intents?;
        for run in &self.live {
            let current = self
                .runtime
                .run(*run)
                .ok_or(Rejection::StorageUnavailable)?;
            let bytes =
                encode_guardian_frame(&current).map_err(|_| Rejection::StorageUnavailable)?;
            connection
                .execute(
                    "UPDATE guardian_runs SET snapshot=?1 WHERE run=?2 AND snapshot!=?1",
                    params![&bytes[4..], run.to_string()],
                )
                .map_err(|_| Rejection::StorageUnavailable)?;
        }
        // An accepted attempt without a live handle prevents claiming completed scope cleanup.
        let unknown: i64 = connection
            .query_row("SELECT count(*) FROM guardian_runs", [], |row| {
                row.get(/*idx*/ 0)
            })
            .map_err(|_| Rejection::StorageUnavailable)?;
        if unknown == self.live.len() as i64 {
            let state = encode_guardian_frame(&self.runtime.state())
                .map_err(|_| Rejection::StorageUnavailable)?;
            connection
                .execute(
                    "UPDATE guardian_run_scope SET state=?1 WHERE singleton=1 AND state!=?1",
                    [&state[4..]],
                )
                .map_err(|_| Rejection::StorageUnavailable)?;
        }
        Ok(())
    }

    /// Reapplies durable force intent so a lost reply never requires the caller to keep polling.
    fn restore_stop_intents(&mut self, connection: &Connection) -> Result<(), Rejection> {
        if !connection.is_autocommit() {
            return Err(Rejection::StorageUnavailable);
        }
        if scope_state(connection)? != ScopeState::Open {
            self.runtime
                .close(StopRequest::Force, Instant::now())
                .map_err(|_| Rejection::StorageUnavailable)?;
        }
        for run in &self.live {
            let stop: bool = connection
                .query_row(
                    "SELECT stop_requested FROM guardian_runs WHERE run=?1",
                    [run.to_string()],
                    |row| row.get(/*idx*/ 0),
                )
                .map_err(|_| Rejection::StorageUnavailable)?;
            if stop {
                self.runtime
                    .stop_run(*run, StopRequest::Force, Instant::now())
                    .map_err(|_| Rejection::StorageUnavailable)?;
            }
        }
        Ok(())
    }
}

/// Missing identities stay missing instead of implicitly accepting a new launch.
fn snapshot(connection: &Connection, run: RunId) -> Result<RunSnapshot, Rejection> {
    let bytes: Vec<u8> = connection
        .query_row(
            "SELECT snapshot FROM guardian_runs WHERE run=?1",
            [run.to_string()],
            |row| row.get(/*idx*/ 0),
        )
        .optional()
        .map_err(|_| Rejection::StorageUnavailable)?
        .ok_or(Rejection::UnknownRun)?;
    decode_guardian_payload(&bytes).map_err(|_| Rejection::StorageUnavailable)
}

/// Admission and terminal scope facts come from the same journal as the accepted attempts.
fn scope_state(connection: &Connection) -> Result<ScopeState, Rejection> {
    let bytes: Vec<u8> = connection
        .query_row(
            "SELECT state FROM guardian_run_scope WHERE singleton=1",
            [],
            |row| row.get(/*idx*/ 0),
        )
        .map_err(|_| Rejection::StorageUnavailable)?;
    decode_guardian_payload(&bytes).map_err(|_| Rejection::StorageUnavailable)
}
