//! Checkout admission rules and durable per-session refusal records.
use super::*;

/// Bounds the independent plugin processes that may share one checkout.
pub const MAX_SESSIONS_PER_CHECKOUT: u32 = 8;

/// Expected checkout refusals that terminate only the incoming Agent session.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SessionRefusal {
    #[error("checkout is busy with a restore or an unfinished session")]
    CheckoutBusy,
    #[error("checkout already has the maximum number of unfinished sessions")]
    CheckoutSessionLimit,
}

impl SessionRefusal {
    /// Stable machine-readable detail carried by the refused session's terminal event.
    pub fn detail(self) -> &'static str {
        match self {
            Self::CheckoutBusy => "checkout_busy",
            Self::CheckoutSessionLimit => "checkout_session_limit",
        }
    }

    /// Recognizes a session terminal that ended in a checkout refusal.
    ///
    /// Delivery uses this to prove a completed execution was refused at admission rather than
    /// run to its end, so the checkout cannot be snapshotted as this session's own result.
    pub fn from_detail(detail: &str) -> Option<Self> {
        match detail {
            "checkout_busy" => Some(Self::CheckoutBusy),
            "checkout_session_limit" => Some(Self::CheckoutSessionLimit),
            _ => None,
        }
    }
}

/// A start that admission refused, carrying the durable projection of its input.
///
/// Admission is the single owner of that projection: it strips runtime-only MCP Setting values
/// before reading or writing anything, and hands the projected input out only inside this value.
/// [`NodeDatabase::refuse_session`] accepts nothing else, so a refused execution can never
/// persist the raw control frame, and its record compares equal to any retransmission of the
/// same start whatever Setting values that retransmission carries.
#[derive(Debug, thiserror::Error)]
#[error("{refusal}")]
pub struct RefusedSession {
    pub(super) refusal: SessionRefusal,
    pub(super) command: Box<StartAgentSessionMessage>,
}

impl RefusedSession {
    /// Why the checkout refused this start.
    pub fn refusal(&self) -> SessionRefusal {
        self.refusal
    }
}

impl<G: WriteGuard> NodeDatabase<G> {
    /// Records a bare session refusal observed by [`NodeDatabase::accept_session`].
    ///
    /// Refusal is deliberately bare-only: a controlled start runs behind the runtime singleton,
    /// which refuses any unfinished execution with `ResourceConflict` before a checkout could
    /// be classified, and that older contract — including its connection close — stays as is.
    pub fn refuse_session(
        &mut self,
        refused: &RefusedSession,
        node: &NodeRuntimeIdentity,
    ) -> Result<SessionExecution, Error> {
        let (command, refusal) = (refused.command.as_ref(), refused.refusal);
        if node.node_id != self.node_id {
            return Err(Error::NodeMismatch);
        }
        if let Some(record) = self.find_session(&command.operation_id, &command.execution_id)? {
            return if record.command == *command {
                Ok(record)
            } else {
                Err(Error::IdentityConflict)
            };
        }
        let result = AgentSessionResult::AgentSessionEnded(AgentSessionEnded {
            node: node.clone(),
            reason: AgentSessionEndReason::AgentFailed,
            detail: Some(refusal.detail().to_owned()),
            plugin_id: None,
            setting_id: None,
            transport: None,
            scheduled_result: None,
            artifact_manifest: None,
        });
        let event = NodeToControllerMessage::AgentSessionEnded(AgentSessionEndedMessage {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            operation_id: command.operation_id.clone(),
            execution_id: command.execution_id.clone(),
            sequence: Sequence::new(/*value*/ 1),
            payload: result.clone(),
        });
        event.validate()?;
        // The refusal was decided inside admission's immediate transaction; this record only
        // makes that decision durable with the runtime identity, so the reason is kept exactly
        // as classified even if a sibling's terminal lands in between.
        self.guard.before_write(WritePoint::Accept)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "INSERT INTO execution_identities VALUES(?1,?2,'agent_session')",
            params![command.operation_id.as_str(), command.execution_id.as_str()],
        )?;
        self.guard.before_write(WritePoint::Complete)?;
        tx.execute(
            "INSERT INTO node_executions(execution,kind,input,state,result,last_sequence) VALUES(?1,'agent_session',?2,'completed',?3,1)",
            params![
                command.execution_id.as_str(),
                serde_json::to_string(command)?,
                serde_json::to_string(&result)?
            ],
        )?;
        if command.payload.spec.prior_revision.is_some() {
            tx.execute(
                "INSERT INTO session_restores(execution,phase) VALUES(?1,'pending')",
                [command.execution_id.as_str()],
            )?;
        }
        self.guard.before_write(WritePoint::Outbox)?;
        tx.execute(
            "INSERT INTO execution_events VALUES(?1,1,?2)",
            params![
                command.execution_id.as_str(),
                serde_json::to_string(&event)?
            ],
        )?;
        tx.commit()?;
        Ok(SessionExecution {
            command: command.clone(),
            state: ExecutionState::Completed(ExecutionResult::AgentSession(result)),
            last_sequence: 1,
        })
    }
}

/// Classifies only checkout-owned refusals; runtime-wide conflicts remain `ResourceConflict`.
///
/// The caller runs this inside the same `IMMEDIATE` transaction as the acceptance insert, so
/// the fences and the count are decided against exactly the state the insert will follow.
pub(super) fn checkout_refusal(
    connection: &rusqlite::Connection,
    command: &StartAgentSessionMessage,
) -> Result<Option<SessionRefusal>, Error> {
    let checkout = command.payload.spec.checkout_execution_id.as_str();
    if command.payload.spec.prior_revision.is_some()
        && connection.query_row("SELECT EXISTS(SELECT 1 FROM session_stop_attempts WHERE checkout=?1 AND state IN ('stop_unconfirmed','legacy_unknown'))", [checkout], |r| r.get::<_,bool>(0))? {
        return Ok(Some(SessionRefusal::CheckoutBusy));
    }
    let (unknown_or_restoring, unfinished): (bool, i64) = connection.query_row(
        "SELECT
            EXISTS(
                SELECT 1 FROM node_executions
                LEFT JOIN session_restores USING(execution)
                WHERE json_extract(input,'$.payload.spec.checkout_execution_id')=?1
                  AND (
                    session_restores.phase='legacy_unknown'
                    OR EXISTS(
                        SELECT 1 FROM restore_process_attempts
                        WHERE restore_process_attempts.execution=node_executions.execution AND cleaned=0
                    )
                    OR (node_executions.state<>'completed' AND session_restores.phase IN ('pending','git_admitted','restored'))
                  )
            ),
            (SELECT COUNT(*) FROM node_executions
             WHERE state<>'completed' AND json_extract(input,'$.payload.spec.checkout_execution_id')=?1)",
        [checkout],
        |row| Ok((row.get(/*idx*/ 0)?, row.get(/*idx*/ 1)?)),
    )?;
    let unfinished = u32::try_from(unfinished).map_err(|_| Error::InvalidSchema)?;
    if unknown_or_restoring || (command.payload.spec.prior_revision.is_some() && unfinished > 0) {
        return Ok(Some(SessionRefusal::CheckoutBusy));
    }
    if unfinished >= MAX_SESSIONS_PER_CHECKOUT {
        return Ok(Some(SessionRefusal::CheckoutSessionLimit));
    }
    Ok(None)
}
