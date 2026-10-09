//! Durable session responsibility and a shareable actor-facing journal under the Node's lease.
mod commands;
mod events;
mod journal;
mod model;
pub use model::{
    CommandAdmission, SessionCommandInput, SessionCommandSettlement, SessionCommandState,
    SessionExecution,
};

use crate::*;
use ora_node_protocol::*;
use rusqlite::{OptionalExtension, params};
use std::sync::{Arc, Mutex, MutexGuard};

/// Narrow actor connection sharing the Node's exclusive lease; cloned handles serialize writes.
/// Separate journals and the admission connection coordinate through SQLite transactions.
pub struct SessionJournal<G = DurableWrites> {
    inner: Arc<Mutex<NodeDatabase<G>>>,
}

impl<G> Clone for SessionJournal<G> {
    /// Cloning never opens another Node owner or loses the original lease.
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<G: WriteGuard> SessionJournal<G> {
    /// A poisoned actor handle fails closed rather than guessing whether a write committed.
    fn lock(&self) -> Result<MutexGuard<'_, NodeDatabase<G>>, Error> {
        self.inner.lock().map_err(|_| Error::InvalidTransition)
    }
}

impl<G: WriteGuard> NodeDatabase<G> {
    /// Creates an actor connection without reacquiring the process-wide database lease.
    pub fn session_journal(&self) -> Result<SessionJournal<G>, Error> {
        let connection =
            rusqlite::Connection::open(self.connection.path().ok_or(Error::InvalidSchema)?)?;
        connection.pragma_update(/*schema_name*/ None, "foreign_keys", "ON")?;
        connection.pragma_update(/*schema_name*/ None, "synchronous", "FULL")?;
        Ok(SessionJournal {
            inner: Arc::new(Mutex::new(NodeDatabase {
                connection,
                guard: Arc::clone(&self.guard),
                node_id: self.node_id.clone(),
                _lease: Arc::clone(&self._lease),
            })),
        })
    }

    /// Resolves both identity keys before reading session-owned data.
    pub fn find_session(
        &self,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<Option<SessionExecution>, Error> {
        if self
            .identity_kind(operation, execution)?
            .is_some_and(|kind| kind != "agent_session")
        {
            return Err(Error::IdentityConflict);
        }
        read(&self.connection, execution)
    }

    /// Admits a bare input only in a home that has never enabled runtime control.
    pub fn accept_session(
        &mut self,
        command: &StartAgentSessionMessage,
    ) -> Result<SessionExecution, Error> {
        let fenced: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM runtime_enforcement) OR EXISTS(SELECT 1 FROM runtime_binding)", [], |r| r.get(/*idx*/ 0))?;
        if fenced {
            return Err(Error::InvalidTransition);
        }
        self.accept_session_input(command, /*permit*/ None)
    }

    /// Stores Cloud's exact execution responsibility with its immutable session input.
    pub fn accept_controlled_session(
        &mut self,
        command: &StartAgentSessionMessage,
        permit: &RuntimeBinding,
    ) -> Result<SessionExecution, Error> {
        self.validate_runtime_permit(permit)?;
        if permit.execution_id != command.execution_id.as_str()
            || permit.node_operation_id != command.operation_id.as_str()
        {
            return Err(Error::IdentityConflict);
        }
        self.accept_session_input(command, Some(permit))
    }

    /// Rejects cross-family identity reuse before atomically registering the session.
    fn accept_session_input(
        &mut self,
        command: &StartAgentSessionMessage,
        permit: Option<&RuntimeBinding>,
    ) -> Result<SessionExecution, Error> {
        command.validate()?;
        if command.payload.spec.node_id != self.node_id {
            return Err(Error::NodeMismatch);
        }
        if let Some(record) = self.find_session(&command.operation_id, &command.execution_id)? {
            return if record.command == *command {
                Ok(record)
            } else {
                Err(Error::IdentityConflict)
            };
        }
        if permit.is_some() && !self.unfinished_runtime_executions()?.is_empty() {
            return Err(Error::ResourceConflict);
        }
        self.guard.before_write(WritePoint::Accept)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO execution_identities VALUES(?1,?2,'agent_session')",
            params![command.operation_id.as_str(), command.execution_id.as_str()],
        )?;
        tx.execute("INSERT INTO node_executions(execution,kind,input,state) VALUES(?1,'agent_session',?2,'accepted')", params![command.execution_id.as_str(),serde_json::to_string(command)?])?;
        if let Some(permit) = permit {
            tx.execute(
                "INSERT INTO execution_control(execution,permit) VALUES(?1,?2)",
                params![
                    command.execution_id.as_str(),
                    serde_json::to_string(permit)?
                ],
            )?;
        }
        tx.commit()?;
        Ok(SessionExecution {
            command: command.clone(),
            state: ExecutionState::Accepted,
            last_sequence: 0,
        })
    }

    /// Rechecks authority at start. A running session is never authorized to restart from its input.
    pub fn start_session(
        &mut self,
        command: &StartAgentSessionMessage,
        incarnation: &NodeIncarnationId,
    ) -> Result<bool, Error> {
        let record = self
            .find_session(&command.operation_id, &command.execution_id)?
            .ok_or(Error::IdentityConflict)?;
        if record.command != *command || record.state != ExecutionState::Accepted {
            return Err(Error::InvalidTransition);
        }
        if !self
            .start_controlled_execution_for(&command.execution_id, Some(incarnation.as_str()))?
        {
            return Ok(false);
        }
        self.guard.before_write(WritePoint::Progress)?;
        let changed = self.connection.execute(
            "UPDATE node_executions SET state='running' WHERE execution=?1 AND state='accepted'",
            [command.execution_id.as_str()],
        )?;
        if changed != 1 {
            return Err(Error::InvalidTransition);
        }
        Ok(true)
    }

    /// Lists unfinished sessions for interrupted settlement, never for automatic Agent restart.
    pub fn recoverable_sessions(&self) -> Result<Vec<SessionExecution>, Error> {
        self.connection.prepare("SELECT input,state,result,last_sequence FROM node_executions WHERE kind='agent_session' AND state<>'completed' ORDER BY rowid")?
            .query_map([], row)?.map(|r| decode(r?)).collect()
    }
}

/// Loads a session while the caller owns the connection or transaction lock.
fn read(
    connection: &rusqlite::Connection,
    execution: &ExecutionId,
) -> Result<Option<SessionExecution>, Error> {
    connection
        .query_row(
            "SELECT input,state,result,last_sequence FROM node_executions WHERE execution=?1",
            [execution.as_str()],
            row,
        )
        .optional()?
        .map(decode)
        .transpose()
}

type SessionRow = (String, String, Option<String>, i64);

/// Keeps SQL row decoding shared between admission, actor writes and recovery.
fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok((
        row.get(/*idx*/ 0)?,
        row.get(/*idx*/ 1)?,
        row.get(/*idx*/ 2)?,
        row.get(/*idx*/ 3)?,
    ))
}

/// Refuses malformed durable state rather than interpreting it as a running session.
fn decode((input, state, result, last_sequence): SessionRow) -> Result<SessionExecution, Error> {
    let state = match (state.as_str(), result) {
        ("accepted", None) => ExecutionState::Accepted,
        ("running", None) => ExecutionState::Running,
        ("completed", Some(result)) => ExecutionState::Completed(ExecutionResult::AgentSession(
            serde_json::from_str(&result)?,
        )),
        _ => return Err(Error::InvalidSchema),
    };
    Ok(SessionExecution {
        command: serde_json::from_str(&input)?,
        state,
        last_sequence: u64::try_from(last_sequence).map_err(|_| Error::InvalidSchema)?,
    })
}
