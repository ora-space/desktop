//! Command admission and settlement are serialized against session termination.
use super::*;

impl<G: WriteGuard> NodeDatabase<G> {
    /// Persists a validated command before its accepted reply or actor wake. The protocol caller
    /// must establish current runtime authority before invoking this storage operation.
    pub fn accept_session_command(
        &mut self,
        input: &SessionCommandInput,
    ) -> Result<CommandAdmission, Error> {
        input.validate()?;
        if input.node_id() != &self.node_id {
            return Err(Error::NodeMismatch);
        }
        if self
            .identity_kind(input.operation_id(), input.execution_id())?
            .as_deref()
            != Some("agent_session")
        {
            return Err(Error::IdentityConflict);
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let session = read(&tx, input.execution_id())?.ok_or(Error::IdentityConflict)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT input FROM session_commands WHERE execution=?1 AND command_id=?2",
                params![input.execution_id().as_str(), input.command_id().as_str()],
                |r| r.get(/*idx*/ 0),
            )
            .optional()?;
        if let Some(existing) = &existing
            && serde_json::from_str::<SessionCommandInput>(existing)? != *input
        {
            return Err(Error::IdentityConflict);
        }
        // Even a retransmission after termination receives the protocol's bounded rejection.
        if matches!(session.state, ExecutionState::Completed(_)) {
            return Ok(CommandAdmission::SessionEnded);
        }
        if existing.is_some() {
            return Ok(CommandAdmission::Accepted);
        }
        self.guard.before_write(WritePoint::Accept)?;
        tx.execute("INSERT INTO session_commands(execution,command_id,input,state) VALUES(?1,?2,?3,'queued')", params![input.execution_id().as_str(),input.command_id().as_str(),serde_json::to_string(input)?])?;
        tx.commit()?;
        Ok(CommandAdmission::Accepted)
    }
}

impl<G: WriteGuard> SessionJournal<G> {
    /// Returns the whole durable queue, including an EndSession behind pending user turns.
    pub fn queued_commands(
        &self,
        execution: &ExecutionId,
    ) -> Result<Vec<SessionCommandInput>, Error> {
        let db = self.lock()?;
        if read(&db.connection, execution)?.is_none() {
            return Err(Error::IdentityConflict);
        }
        db.connection.prepare("SELECT input FROM session_commands WHERE execution=?1 AND state='queued' ORDER BY acceptance_order")?
            .query_map([execution.as_str()], |r| r.get::<_,String>(0))?
            .map(|row| Ok(serde_json::from_str(&row?)?)).collect()
    }

    /// Whether the session still runs as far as the ledger knows: it has no terminal result and
    /// no `EndSession` was ever accepted for it, whether or not the session acted on it yet.
    /// Local work done on the session's behalf (`ora-git`) stops being accepted from the moment
    /// an end is requested, not only once the session has ended.
    pub fn session_running(&self, execution: &ExecutionId) -> Result<bool, Error> {
        let db = self.lock()?;
        let Some(session) = read(&db.connection, execution)? else {
            return Ok(false);
        };
        if matches!(session.state, ExecutionState::Completed(_)) {
            return Ok(false);
        }
        let inputs = db
            .connection
            .prepare("SELECT input FROM session_commands WHERE execution=?1")?
            .query_map([execution.as_str()], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for input in inputs {
            if matches!(
                serde_json::from_str::<SessionCommandInput>(&input)?,
                SessionCommandInput::EndSession(_)
            ) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Observes settlement without consuming the command or changing its admission order.
    pub fn command_state(
        &self,
        execution: &ExecutionId,
        command: &CommandId,
    ) -> Result<Option<SessionCommandState>, Error> {
        let db = self.lock()?;
        let state: Option<String> = db
            .connection
            .query_row(
                "SELECT state FROM session_commands WHERE execution=?1 AND command_id=?2",
                params![execution.as_str(), command.as_str()],
                |r| r.get(/*idx*/ 0),
            )
            .optional()?;
        state
            .map(|s| match s.as_str() {
                "queued" => Ok(SessionCommandState::Queued),
                "executed" => Ok(SessionCommandState::Executed),
                "discarded" => Ok(SessionCommandState::Discarded),
                _ => Err(Error::InvalidSchema),
            })
            .transpose()
    }

    /// A queued command transitions exactly once. Retrying the same settlement is idempotent;
    /// a different settlement or an unknown command fails without resurrecting work.
    pub fn settle_command(
        &self,
        execution: &ExecutionId,
        command: &CommandId,
        settlement: super::model::SessionCommandSettlement,
    ) -> Result<(), Error> {
        let mut db = self.lock()?;
        let guard = Arc::clone(&db.guard);
        let tx = db
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let state: Option<String> = tx
            .query_row(
                "SELECT state FROM session_commands WHERE execution=?1 AND command_id=?2",
                params![execution.as_str(), command.as_str()],
                |r| r.get(/*idx*/ 0),
            )
            .optional()?;
        let next = match settlement {
            super::model::SessionCommandSettlement::Executed => "executed",
            super::model::SessionCommandSettlement::Discarded => "discarded",
        };
        match state.as_deref() {
            Some("queued") => {}
            Some(existing) if existing == next => return Ok(()),
            Some(_) => return Err(Error::InvalidTransition),
            None => return Err(Error::IdentityConflict),
        }
        guard.before_write(WritePoint::Progress)?;
        tx.execute(
            "UPDATE session_commands SET state=?1 WHERE execution=?2 AND command_id=?3",
            params![next, execution.as_str(), command.as_str()],
        )?;
        tx.commit()?;
        Ok(())
    }
}

impl<G: WriteGuard> NodeDatabase<G> {
    /// New input needs a live binding for the execution's original control scope. A new lease
    /// may refresh the same control epoch, but never adopt another user's unfinished session.
    pub fn authorize_session_command(
        &self,
        execution: &ExecutionId,
        incarnation: &NodeIncarnationId,
    ) -> Result<(), Error> {
        let original: Option<String> = self
            .connection
            .query_row(
                "SELECT permit FROM execution_control WHERE execution=?1",
                [execution.as_str()],
                |row| row.get(/*idx*/ 0),
            )
            .optional()?;
        let Some(original) = original else {
            let enforced: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM runtime_enforcement) OR EXISTS(SELECT 1 FROM runtime_binding)", [], |row| row.get(/*idx*/ 0))?;
            return if enforced {
                Err(Error::InvalidTransition)
            } else {
                Ok(())
            };
        };
        let original: RuntimeBinding = serde_json::from_str(&original)?;
        let mut current = self.runtime_binding()?.ok_or(Error::InvalidTransition)?;
        if current.node_incarnation_id != incarnation.as_str()
            || current.node_incarnation_id != original.node_incarnation_id
            || current.tenant_id != original.tenant_id
            || current.workspace_id != original.workspace_id
            || current.sandbox_id != original.sandbox_id
            || current.runtime_generation != original.runtime_generation
            || current.control_epoch != original.control_epoch
            || current.session_id != original.session_id
            || current.actor_user_id != original.actor_user_id
            || current.operation_id != original.operation_id
        {
            return Err(Error::InvalidTransition);
        }
        current.execution_id = original.execution_id;
        current.node_operation_id = original.node_operation_id;
        self.validate_runtime_permit(&current)
    }
}
