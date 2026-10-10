//! Revision delivery responsibility: input, the plan frozen before the first upload, and the one
//! terminal event retained until acknowledged.
//!
//! Deliveries live in their own table rather than in `node_executions`: they share neither Thread
//! events nor session commands, and keeping them apart means session recovery can never mistake a
//! delivery for an interrupted session. Upload grants never reach this module.
use crate::*;
use ora_node_protocol::*;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

/// What a prepared delivery declares once every frozen object is uploaded.
///
/// Only the two successful outcomes can be frozen; a failure is terminal at once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "result", rename_all = "snake_case")]
pub enum FrozenOutcome {
    Delivered(RevisionDelivered),
    Unchanged(RevisionUnchanged),
}

impl FrozenOutcome {
    /// The terminal result reported unchanged after upload, even after restarts.
    pub fn result(&self) -> RevisionExecutionResult {
        match self {
            Self::Delivered(result) => RevisionExecutionResult::RevisionDelivered(result.clone()),
            Self::Unchanged(result) => RevisionExecutionResult::RevisionUnchanged(result.clone()),
        }
    }
}

/// Object bytes and declaration fixed before the first PUT; restarts reuse exactly these.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryPlan {
    /// Single directory name under the Node's delivery root that holds the frozen objects.
    pub directory: String,
    pub outcome: FrozenOutcome,
}

/// Where a running delivery is; a frozen plan never returns to preparation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveryProgress {
    /// Nothing durable beyond the input: a restart prepares again from the checkout.
    Preparing,
    /// Uploading the frozen objects.
    Frozen(DeliveryPlan),
    Completed(RevisionExecutionResult),
}

impl DeliveryProgress {
    /// Preparation and upload are both `Running`; the Controller waits on the terminal event.
    pub fn state(&self) -> ExecutionState {
        match self {
            Self::Preparing | Self::Frozen(_) => ExecutionState::Running,
            Self::Completed(result) => {
                ExecutionState::Completed(ExecutionResult::Revision(Box::new(result.clone())))
            }
        }
    }
}

/// One accepted delivery with its original input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevisionDelivery {
    pub command: DeliverRevisionMessage,
    pub progress: DeliveryProgress,
}

impl<G: WriteGuard> NodeDatabase<G> {
    /// Reads by both identity keys so another execution family cannot adopt the identity.
    pub fn find_delivery(
        &self,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<Option<RevisionDelivery>, Error> {
        if self
            .identity_kind(operation, execution)?
            .is_some_and(|kind| kind != "deliver_revision")
        {
            return Err(Error::IdentityConflict);
        }
        read(&self.connection, execution)
    }

    /// Admits a bare input only in a home that has never enabled runtime control.
    pub fn accept_delivery(
        &mut self,
        command: &DeliverRevisionMessage,
    ) -> Result<RevisionDelivery, Error> {
        let fenced: bool = self.connection.query_row("SELECT EXISTS(SELECT 1 FROM runtime_enforcement) OR EXISTS(SELECT 1 FROM runtime_binding)", [], |r| r.get(/*idx*/ 0))?;
        if fenced {
            return Err(Error::InvalidTransition);
        }
        self.accept_delivery_input(command, /*permit*/ None)
    }

    /// Stores Cloud's exact execution responsibility together with the immutable input.
    pub fn accept_controlled_delivery(
        &mut self,
        envelope: &ControlledDeliverRevision,
    ) -> Result<RevisionDelivery, Error> {
        envelope.validate()?;
        self.validate_runtime_permit(&envelope.binding)?;
        self.accept_delivery_input(&envelope.command, Some(&envelope.binding))
    }

    /// Deduplicates the full input, then records the started delivery and its control atomically.
    ///
    /// The permit was just checked live, so the delivery is recorded as started: a later restart
    /// resumes it under a new incarnation instead of re-qualifying it, because a delivery must
    /// finish with the bytes it froze (its failure codes have no "interrupted").
    fn accept_delivery_input(
        &mut self,
        command: &DeliverRevisionMessage,
        permit: Option<&RuntimeBinding>,
    ) -> Result<RevisionDelivery, Error> {
        command.validate()?;
        if command.payload.spec.node_id != self.node_id {
            return Err(Error::NodeMismatch);
        }
        if let Some(record) = self.find_delivery(&command.operation_id, &command.execution_id)? {
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
            "INSERT INTO execution_identities VALUES(?1,?2,'deliver_revision')",
            params![command.operation_id.as_str(), command.execution_id.as_str()],
        )?;
        tx.execute(
            "INSERT INTO revision_deliveries(execution,input,state) VALUES(?1,?2,'running')",
            params![
                command.execution_id.as_str(),
                serde_json::to_string(command)?
            ],
        )?;
        if let Some(permit) = permit {
            tx.execute(
                "INSERT INTO execution_control(execution,permit,started) VALUES(?1,?2,1)",
                params![
                    command.execution_id.as_str(),
                    serde_json::to_string(permit)?
                ],
            )?;
        }
        tx.commit()?;
        Ok(RevisionDelivery {
            command: command.clone(),
            progress: DeliveryProgress::Preparing,
        })
    }

    /// Persists the plan before the first upload. Only a preparing delivery may freeze, and the
    /// declaration must name exactly the input's ref, base and keys.
    pub fn freeze_delivery(
        &mut self,
        execution: &ExecutionId,
        plan: &DeliveryPlan,
    ) -> Result<(), Error> {
        let record = read(&self.connection, execution)?.ok_or(Error::IdentityConflict)?;
        if record.progress != DeliveryProgress::Preparing {
            return Err(Error::InvalidTransition);
        }
        let spec = &record.command.payload.spec;
        let consistent = match &plan.outcome {
            FrozenOutcome::Delivered(result) => {
                result.node.node_id == self.node_id
                    && result.base_commit == spec.base_commit
                    && result.revision_ref == spec.revision_ref
                    && result.bundle.key == spec.bundle_key
                    && result.history.key == spec.history_key
            }
            // Unchanged means no new commit beyond the base or, for a resumed session, beyond
            // the prior Revision's final commit; any other commit would lose work unbundled.
            FrozenOutcome::Unchanged(result) => {
                result.node.node_id == self.node_id
                    && result.base_commit == spec.base_commit
                    && result.revision_ref == spec.revision_ref
                    && result.history.key == spec.history_key
                    && (result.final_commit == spec.base_commit
                        || spec
                            .prior_revision
                            .as_ref()
                            .is_some_and(|prior| prior.final_commit == result.final_commit))
            }
        };
        if !consistent
            || plan.directory.is_empty()
            || std::path::Path::new(&plan.directory).file_name()
                != Some(std::ffi::OsStr::new(&plan.directory))
        {
            return Err(Error::IdentityConflict);
        }
        event(&record.command, plan.outcome.result()).validate()?;
        self.guard.before_write(WritePoint::Progress)?;
        let changed = self.connection.execute(
            "UPDATE revision_deliveries SET plan=?1 WHERE execution=?2 AND state='running' AND plan IS NULL",
            params![serde_json::to_string(plan)?, execution.as_str()],
        )?;
        if changed != 1 {
            return Err(Error::InvalidTransition);
        }
        Ok(())
    }

    /// Commits the one terminal result and its replay envelope together, dropping the plan.
    pub fn complete_delivery(
        &mut self,
        execution: &ExecutionId,
        result: RevisionExecutionResult,
    ) -> Result<(), Error> {
        let record = read(&self.connection, execution)?.ok_or(Error::IdentityConflict)?;
        if matches!(record.progress, DeliveryProgress::Completed(_)) {
            return Err(Error::InvalidTransition);
        }
        let node = match &result {
            RevisionExecutionResult::RevisionDelivered(r) => &r.node,
            RevisionExecutionResult::RevisionUnchanged(r) => &r.node,
            RevisionExecutionResult::RevisionFailed(r) => &r.node,
        };
        if node.node_id != self.node_id {
            return Err(Error::NodeMismatch);
        }
        let message = event(&record.command, result.clone());
        message.validate()?;
        self.guard.before_write(WritePoint::Complete)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE revision_deliveries SET state='completed',plan=NULL,result=?1 WHERE execution=?2",
            params![serde_json::to_string(&result)?, execution.as_str()],
        )?;
        self.guard.before_write(WritePoint::Outbox)?;
        tx.execute(
            "INSERT INTO revision_outbox VALUES(?1,?2)",
            params![
                execution.as_str(),
                serde_json::to_string(&NodeToControllerMessage::RevisionResult(message))?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Lists unfinished deliveries in admission order; each resumes from its durable progress.
    pub fn recoverable_deliveries(&self) -> Result<Vec<RevisionDelivery>, Error> {
        self.connection
            .prepare("SELECT input,state,plan,result FROM revision_deliveries WHERE state<>'completed' ORDER BY rowid")?
            .query_map([], row)?
            .map(|r| decode(r?))
            .collect()
    }

    /// Resolves the session a delivery names by execution alone, as the delivery input carries
    /// no session operation. Another family's execution is not a session.
    pub fn delivery_session(
        &self,
        execution: &ExecutionId,
    ) -> Result<Option<SessionExecution>, Error> {
        let operation: Option<String> = self
            .connection
            .query_row(
                "SELECT operation FROM execution_identities WHERE execution=?1 AND kind='agent_session'",
                [execution.as_str()],
                |r| r.get(/*idx*/ 0),
            )
            .optional()?;
        match operation {
            Some(operation) => self.find_session(&OperationId::new(operation), execution),
            None => Ok(None),
        }
    }

    /// Returns the checkout path and commit of a completed successful clone; any other clone
    /// state, or another family's execution, leaves the delivery without a checkout.
    pub fn delivery_checkout(
        &self,
        execution: &ExecutionId,
    ) -> Result<Option<(std::path::PathBuf, CommitId)>, Error> {
        let row: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT target,progress FROM clone_executions WHERE execution=?1",
                [execution.as_str()],
                |r| Ok((r.get(/*idx*/ 0)?, r.get(/*idx*/ 1)?)),
            )
            .optional()?;
        let Some((target, progress)) = row else {
            return Ok(None);
        };
        match serde_json::from_str::<CloneProgress>(&progress)? {
            CloneProgress::Completed(CloneExecutionResult::CloneReady(ready)) => Ok(Some((
                serde_json::from_str::<CloneTarget>(&target)?.path,
                ready.commit,
            ))),
            CloneProgress::Completed(CloneExecutionResult::CloneFailed(_))
            | CloneProgress::Pending(_)
            | CloneProgress::Unknown(_) => Ok(None),
        }
    }

    /// Only the exact terminal sequence of a completed delivery releases its event.
    pub(crate) fn acknowledge_delivery(&mut self, ack: &EventAckMessage) -> Result<(), Error> {
        let record = self
            .find_delivery(&ack.operation_id, &ack.execution_id)?
            .ok_or(Error::InvalidAck)?;
        if ack.sequence != Sequence::new(/*value*/ 1)
            || !matches!(record.progress, DeliveryProgress::Completed(_))
        {
            return Err(Error::InvalidAck);
        }
        self.guard.before_write(WritePoint::Acknowledge)?;
        self.connection.execute(
            "DELETE FROM revision_outbox WHERE execution=?1",
            [ack.execution_id.as_str()],
        )?;
        Ok(())
    }
}

/// Builds the single terminal envelope; replay reuses the stored copy and never rebuilds it.
fn event(
    command: &DeliverRevisionMessage,
    payload: RevisionExecutionResult,
) -> RevisionResultMessage {
    RevisionResultMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: command.operation_id.clone(),
        execution_id: command.execution_id.clone(),
        sequence: Sequence::new(/*value*/ 1),
        payload,
    }
}

/// Loads one delivery while the caller owns the connection.
fn read(
    connection: &rusqlite::Connection,
    execution: &ExecutionId,
) -> Result<Option<RevisionDelivery>, Error> {
    connection
        .query_row(
            "SELECT input,state,plan,result FROM revision_deliveries WHERE execution=?1",
            [execution.as_str()],
            row,
        )
        .optional()?
        .map(decode)
        .transpose()
}

type DeliveryRow = (String, String, Option<String>, Option<String>);

/// Shares SQL row extraction between lookups and recovery scans.
fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeliveryRow> {
    Ok((
        row.get(/*idx*/ 0)?,
        row.get(/*idx*/ 1)?,
        row.get(/*idx*/ 2)?,
        row.get(/*idx*/ 3)?,
    ))
}

/// Refuses malformed rows rather than guessing whether bytes were frozen.
fn decode((input, state, plan, result): DeliveryRow) -> Result<RevisionDelivery, Error> {
    let progress = match (state.as_str(), plan, result) {
        ("running", None, None) => DeliveryProgress::Preparing,
        ("running", Some(plan), None) => DeliveryProgress::Frozen(serde_json::from_str(&plan)?),
        ("completed", None, Some(result)) => {
            DeliveryProgress::Completed(serde_json::from_str(&result)?)
        }
        _ => return Err(Error::InvalidSchema),
    };
    Ok(RevisionDelivery {
        command: serde_json::from_str(&input)?,
        progress,
    })
}
