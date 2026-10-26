use crate::*;
use ora_node_protocol::{RuntimeBinding, ValidateMessage};
use rusqlite::{OptionalExtension, params};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> Result<i64, Error> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::InvalidTransition)?
        .as_millis();
    i64::try_from(millis).map_err(|_| Error::InvalidTransition)
}

// PostgreSQL issues deadlines. A two-second conservative margin refuses clocks that lag the
// issuer and closes early instead of extending authority through transport delay or clock skew.
fn live(permit: &RuntimeBinding) -> Result<(), Error> {
    let now = now_ms()?;
    if permit.input_closed
        || permit.issued_at_ms > now + 2_000
        || now - permit.issued_at_ms > 10_000
        || permit.expires_at_ms <= now + 2_000
    {
        return Err(Error::InvalidTransition);
    }
    Ok(())
}

/// The two delivery families Cloud admits for one run at the same time.
///
/// Cloud keeps a session exclusive with every run execution and each family exclusive with itself,
/// but it dispatches a Revision delivery next to the scheduled-artifact upload of the same run. A
/// Node that treated either family as "the one controlled execution" refused the later arrival and
/// closed the connection, so the families are named instead of being guessed from a table name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryFamily {
    /// The run's Git delivery of a Revision.
    Revision,
    /// The upload of the artifacts one ended scheduled session declared.
    ScheduledArtifacts,
}

impl DeliveryFamily {
    /// The other family Cloud lets run beside this one; its unfinished rows do not conflict.
    fn peer(self) -> &'static str {
        match self {
            Self::Revision => "deliver_scheduled_artifacts",
            Self::ScheduledArtifacts => "deliver_revision",
        }
    }
}

impl<G: WriteGuard> NodeDatabase<G> {
    /// Cloud mode is durable. Reopening this home through a legacy embedding cannot bypass it.
    pub fn enforce_runtime_control(&mut self) -> Result<(), Error> {
        self.guard.before_write(WritePoint::Accept)?;
        self.connection
            .execute("INSERT OR IGNORE INTO runtime_enforcement VALUES(1)", [])?;
        Ok(())
    }

    /// A new Node process closes its persisted input before recovery. Old accepted process
    /// responsibility may be settled; the old host incarnation cannot start another process.
    pub fn enforce_runtime_incarnation(&mut self, incarnation: &str) -> Result<(), Error> {
        self.enforce_runtime_control()?;
        if let Some(mut binding) = self.runtime_binding()?
            && binding.node_incarnation_id != incarnation
        {
            binding.input_closed = true;
            self.guard.before_write(WritePoint::Accept)?;
            self.connection.execute(
                "UPDATE runtime_binding SET data=?1,closed=1 WHERE singleton=1",
                [serde_json::to_string(&binding)?],
            )?;
        }
        Ok(())
    }
    pub fn runtime_binding(&self) -> Result<Option<RuntimeBinding>, Error> {
        let data: Option<String> = self
            .connection
            .query_row(
                "SELECT data FROM runtime_binding WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        data.map(|v| serde_json::from_str(&v).map_err(Error::from))
            .transpose()
    }

    /// Installs a monotonic binding before acknowledging it. Closure is irreversible within a
    /// control epoch, including reconnect, later Controller lease and replay of earlier success.
    pub fn bind_runtime(&mut self, binding: &RuntimeBinding) -> Result<Vec<String>, Error> {
        binding.validate()?;
        if binding.node_id != self.node_id.as_str() {
            return Err(Error::NodeMismatch);
        }
        let old = self.runtime_binding()?;
        let unfinished = self.unfinished_runtime_executions()?;
        let retained_workflow_delivery = old.as_ref().is_some_and(|old| {
            old.input_closed
                && !binding.input_closed
                && old.node_incarnation_id != binding.node_incarnation_id
                && binding.control_epoch > old.control_epoch
                && old.tenant_id == binding.tenant_id
                && old.workspace_id == binding.workspace_id
                && old.sandbox_id == binding.sandbox_id
                && old.runtime_generation == binding.runtime_generation
                && old.node_id == binding.node_id
                && old.session_id.is_empty()
                && binding.session_id.is_empty()
                && !old.operation_id.is_empty()
                && old.operation_id == binding.operation_id
                && old.actor_user_id == binding.actor_user_id
        }) && self
            .retained_workflow_deliveries(&unfinished, binding)?;
        if let Some(old) = &old {
            if old.tenant_id != binding.tenant_id
                || old.workspace_id != binding.workspace_id
                || binding.runtime_generation < old.runtime_generation
                || (binding.runtime_generation == old.runtime_generation
                    && old.sandbox_id != binding.sandbox_id)
                || (binding.runtime_generation > old.runtime_generation
                    && (!old.input_closed
                        || !unfinished.is_empty()
                        || binding.control_epoch <= old.control_epoch))
                || binding.controller_epoch < old.controller_epoch
                || binding.control_epoch < old.control_epoch
                || (binding.control_epoch == old.control_epoch
                    && (binding.control_version < old.control_version
                        || (old.input_closed && !binding.input_closed)
                        || old.session_id != binding.session_id
                        || old.operation_id != binding.operation_id
                        || old.actor_user_id != binding.actor_user_id))
                || (binding.control_epoch > old.control_epoch
                    && !unfinished.is_empty()
                    && !retained_workflow_delivery)
            {
                return Err(Error::InvalidTransition);
            }
        } else if !binding.input_closed && !unfinished.is_empty() {
            return Err(Error::ResourceConflict);
        }
        if !binding.input_closed {
            live(binding)?;
        }
        self.guard.before_write(WritePoint::Accept)?;
        self.connection.execute(
            "INSERT INTO runtime_binding VALUES(1,?1,?2) ON CONFLICT(singleton) DO UPDATE SET data=excluded.data,closed=excluded.closed",
            params![serde_json::to_string(binding)?, binding.input_closed],
        )?;
        Ok(unfinished)
    }

    /// Rebinding the same owner never adopts another kind of unfinished writer or rewrites its
    /// original permit. Only registered workflow deliveries with positive stop proof can finish.
    fn retained_workflow_deliveries(
        &self,
        unfinished: &[String],
        binding: &RuntimeBinding,
    ) -> Result<bool, Error> {
        if unfinished.is_empty() {
            return Ok(false);
        }
        let deliveries = self.recoverable_deliveries()?;
        let journal = self.session_journal()?;
        for execution in unfinished {
            let Some(record) = deliveries
                .iter()
                .find(|record| record.command.execution_id.as_str() == execution)
            else {
                return Ok(false);
            };
            let Some(workflow) = &record.command.payload.spec.workflow else {
                return Ok(false);
            };
            if workflow.workflow_run_id != binding.operation_id || workflow.sessions.is_empty() {
                return Ok(false);
            }
            let control: Option<(String, bool)> = self
                .connection
                .query_row(
                    "SELECT permit,started FROM execution_control WHERE execution=?1",
                    [execution],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()?;
            let Some((permit, true)) = control else {
                return Ok(false);
            };
            let permit: RuntimeBinding = serde_json::from_str(&permit)?;
            if permit.tenant_id != binding.tenant_id
                || permit.workspace_id != binding.workspace_id
                || permit.sandbox_id != binding.sandbox_id
                || permit.runtime_generation != binding.runtime_generation
                || permit.node_id != binding.node_id
                || permit.operation_id != binding.operation_id
                || permit.actor_user_id != binding.actor_user_id
                || permit.execution_id != *execution
                || permit.node_operation_id != record.command.operation_id.as_str()
            {
                return Ok(false);
            }
            for attempt in &workflow.sessions {
                let Some(session) = self.delivery_session(&attempt.execution_id)? else {
                    return Ok(false);
                };
                if !matches!(
                    session.state,
                    ora_node_protocol::ExecutionState::Completed(_)
                ) || !journal.writer_stop_confirmed(&attempt.execution_id)?
                    || session.command.payload.spec.checkout_execution_id
                        != record.command.payload.spec.checkout_execution_id
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Reports every unresolved local liability, including historical unbound executions.
    pub fn unfinished_runtime_executions(&self) -> Result<Vec<String>, Error> {
        self.unfinished_executions_beside(/*peer*/ None)
    }

    /// Reports the unfinished executions that refuse accepting a controlled `family` execution.
    ///
    /// Cloud dispatches a Revision delivery and a scheduled-artifact upload of one run at the same
    /// time (`0065_scheduled_artifact_execution_kind.sql`: "allowing a Revision and an upload
    /// together"), while a session still excludes both and a second execution of either family is
    /// still refused. The pair is the only exception; every other liability closes the slot.
    pub fn unfinished_conflicting_executions(
        &self,
        family: DeliveryFamily,
    ) -> Result<Vec<String>, Error> {
        self.unfinished_executions_beside(Some(family.peer()))
    }

    /// One query for both answers: `peer` is the family whose rows do not conflict.
    ///
    /// `?1 IS NOT` also holds for a null `peer`, so the unparameterized answer needs no second
    /// query to drift from this one.
    fn unfinished_executions_beside(&self, peer: Option<&str>) -> Result<Vec<String>, Error> {
        self.connection.prepare("SELECT execution FROM clone_executions WHERE state<>'completed' UNION SELECT execution FROM executions WHERE state<>'completed' UNION SELECT execution FROM plugin_executions WHERE state<>'completed' UNION SELECT execution FROM node_executions WHERE state<>'completed' UNION SELECT execution FROM revision_deliveries WHERE state<>'completed' AND ?1 IS NOT 'deliver_revision' UNION SELECT execution FROM scheduled_artifact_deliveries WHERE state<>'completed' AND ?1 IS NOT 'deliver_scheduled_artifacts' UNION SELECT execution FROM process_attempts WHERE cleaned=0 UNION SELECT execution FROM session_restores WHERE phase='legacy_unknown' ORDER BY execution")?
            .query_map([peer], |r| r.get(0))?.collect::<Result<Vec<_>,_>>().map_err(Error::from)
    }

    pub(crate) fn validate_runtime_permit(&self, permit: &RuntimeBinding) -> Result<(), Error> {
        permit.validate()?;
        live(permit)?;
        let bound = self.runtime_binding()?.ok_or(Error::InvalidTransition)?;
        if bound.input_closed
            || permit.tenant_id != bound.tenant_id
            || permit.workspace_id != bound.workspace_id
            || permit.sandbox_id != bound.sandbox_id
            || permit.runtime_generation != bound.runtime_generation
            || permit.node_id != bound.node_id
            || permit.node_incarnation_id != bound.node_incarnation_id
            || permit.controller_epoch != bound.controller_epoch
            || permit.control_epoch != bound.control_epoch
            || permit.session_id != bound.session_id
            || permit.actor_user_id != bound.actor_user_id
            || permit.operation_id != bound.operation_id
            || permit.execution_id.is_empty()
        {
            return Err(Error::InvalidTransition);
        }
        Ok(())
    }

    /// Checks the actual execution entrance, then records original responsibility before any
    /// filesystem or process work. Started activities may finish after qualification closes.
    pub fn start_controlled_execution(
        &mut self,
        execution: &ora_node_protocol::ExecutionId,
    ) -> Result<bool, Error> {
        self.start_controlled_execution_for(execution, None)
    }
    pub fn start_controlled_execution_for(
        &mut self,
        execution: &ora_node_protocol::ExecutionId,
        incarnation: Option<&str>,
    ) -> Result<bool, Error> {
        let row: Option<(String, bool)> = self
            .connection
            .query_row(
                "SELECT permit,started FROM execution_control WHERE execution=?1",
                [execution.as_str()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((data, started)) = row else {
            let enforced: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM runtime_enforcement)",
                [],
                |r| r.get(0),
            )?;
            return if enforced || self.runtime_binding()?.is_some() {
                Ok(false)
            } else {
                Ok(true)
            };
        };
        let permit: RuntimeBinding = serde_json::from_str(&data)?;
        if incarnation.is_some_and(|current| current != permit.node_incarnation_id) {
            return Ok(false);
        }
        if started {
            return Ok(true);
        }
        match self.validate_runtime_permit(&permit) {
            Ok(()) => {}
            Err(Error::InvalidTransition) => return Ok(false),
            Err(error) => return Err(error),
        }
        self.guard.before_write(WritePoint::Progress)?;
        self.connection.execute(
            "UPDATE execution_control SET started=1 WHERE execution=?1",
            [execution.as_str()],
        )?;
        Ok(true)
    }
}
