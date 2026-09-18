use crate::*;
use ora_node_protocol::*;
use rusqlite::{OptionalExtension, params};

impl<G: WriteGuard> NodeDatabase<G> {
    /// Reads by either identity, rejecting a rebound operation or execution before any resolution.
    pub fn find(
        &self,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<Option<Execution>, Error> {
        if self
            .identity_kind(operation, execution)?
            .is_some_and(|kind| kind != "worktree")
        {
            return Err(Error::IdentityConflict);
        }
        let mut statement = self.connection.prepare(
            "SELECT input,target,progress FROM executions WHERE operation=?1 OR execution=?2",
        )?;
        let mut rows = statement.query(params![operation.as_str(), execution.as_str()])?;
        if let Some(row) = rows.next()? {
            let record = decode(row)?;
            if record.command.operation_id() != operation
                || record.command.execution_id() != execution
            {
                return Err(Error::IdentityConflict);
            }
            Ok(Some(record))
        } else {
            Ok(None)
        }
    }

    /// Performs durable deduplication without reinterpreting changed references or bindings.
    pub fn existing(&self, command: &Command) -> Result<Option<Execution>, Error> {
        let record = self.find(command.operation_id(), command.execution_id())?;
        if record
            .as_ref()
            .is_some_and(|record| record.command != *command)
        {
            return Err(Error::IdentityConflict);
        }
        Ok(record)
    }

    /// Reserves a fresh create target or associates deletion with previously proven ownership.
    pub fn accept(&mut self, command: &Command, target: &Target) -> Result<Execution, Error> {
        command.validate()?;
        if command.spec().node_id != self.node_id {
            return Err(Error::NodeMismatch);
        }
        if let Some(record) = self.existing(command)? {
            return Ok(record);
        }
        self.guard.before_write(WritePoint::Accept)?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let spec = command.spec();
        let resource: Option<String> = tx
            .query_row(
                "SELECT data FROM resources WHERE worktree=?1",
                [spec.worktree_id.as_str()],
                |row| row.get(/*idx*/ 0),
            )
            .optional()?;
        match command {
            Command::Ensure(_) => {
                if resource.is_some() {
                    return Err(Error::ResourceConflict);
                }
                let mut paths = tx.prepare("SELECT path FROM resources WHERE active=1 UNION ALL SELECT path FROM clone_executions")?;
                for path in paths.query_map([], |row| row.get::<_, String>(/*idx*/ 0))? {
                    let path = std::path::PathBuf::from(path?);
                    if path.starts_with(&target.path) || target.path.starts_with(&path) {
                        return Err(Error::ResourceConflict);
                    }
                }
                let conflict: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM resources WHERE active=1 AND (workspace=?1 OR path=?2 OR (repository=?3 AND branch=?4)))", params![spec.workspace_id.as_str(), target.path.to_str(), target.main_path.to_str(), target.branch.as_str()], |r| r.get(/*idx*/ 0))?;
                if conflict {
                    return Err(Error::ResourceConflict);
                }
            }
            Command::Remove(_) => {
                let resource: Resource =
                    serde_json::from_str(&resource.ok_or(Error::ResourceConflict)?)?;
                if !owns(&resource, spec, target) || resource.state == ResourceState::Reserved {
                    return Err(Error::ResourceConflict);
                }
                let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM executions WHERE state != 'completed' AND json_extract(input, '$.message.payload.spec.worktree_id')=?1)", [spec.worktree_id.as_str()], |row| row.get(/*idx*/ 0))?;
                if pending {
                    return Err(Error::ResourceConflict);
                }
            }
        }
        tx.execute(
            "INSERT INTO executions VALUES (?1,?2,?3,?4,'accepted',?5)",
            params![
                command.operation_id().as_str(),
                command.execution_id().as_str(),
                serde_json::to_string(command)?,
                serde_json::to_string(target)?,
                serde_json::to_string(&Progress::Accepted)?
            ],
        )?;
        if matches!(command, Command::Ensure(_)) {
            let resource = Resource {
                spec: spec.clone(),
                target: target.clone(),
                state: ResourceState::Reserved,
            };
            tx.execute(
                "INSERT INTO resources VALUES (?1,?2,?3,?4,?5,1,?6,?7)",
                params![
                    spec.worktree_id.as_str(),
                    spec.workspace_id.as_str(),
                    target.main_path.to_str(),
                    target.path.to_str(),
                    target.branch.as_str(),
                    serde_json::to_string(&resource)?,
                    command.execution_id().as_str()
                ],
            )?;
        }
        tx.commit()?;
        Ok(Execution {
            command: command.clone(),
            target: Some(target.clone()),
            progress: Progress::Accepted,
        })
    }

    /// Records a definitive precondition failure and event without requiring a resolved target.
    pub fn reject(
        &mut self,
        command: &Command,
        result: WorktreeExecutionResult,
    ) -> Result<(), Error> {
        command.validate()?;
        if command.spec().node_id != self.node_id {
            return Err(Error::NodeMismatch);
        }
        if self.existing(command)?.is_some() {
            return Err(Error::InvalidTransition);
        }
        validate_result(command, &result)?;
        if !matches!(
            result,
            WorktreeExecutionResult::Failed(_) | WorktreeExecutionResult::RemovalFailed(_)
        ) {
            return Err(Error::InvalidTransition);
        }
        self.guard.before_write(WritePoint::Accept)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO executions VALUES (?1,?2,?3,NULL,'completed',?4)",
            params![
                command.operation_id().as_str(),
                command.execution_id().as_str(),
                serde_json::to_string(command)?,
                serde_json::to_string(&Progress::Completed {
                    result: result.clone()
                })?
            ],
        )?;
        self.guard.before_write(WritePoint::Outbox)?;
        tx.execute(
            "INSERT INTO outbox VALUES (?1,?2)",
            params![
                command.execution_id().as_str(),
                serde_json::to_string(&command.event(result))?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Uses compare-and-swap evidence so stale callers cannot overwrite later progress or results.
    pub fn advance(&mut self, record: &Execution, progress: Progress) -> Result<Execution, Error> {
        if matches!(record.progress, Progress::Completed { .. })
            || !matches!(
                progress,
                Progress::Running { .. } | Progress::Unknown { .. }
            )
        {
            return Err(Error::InvalidTransition);
        }
        if let Progress::Running {
            stage, observer, ..
        }
        | Progress::Unknown {
            stage, observer, ..
        } = &progress
            && (observer.node_id != self.node_id
                || observer.incarnation_id.as_str().trim().is_empty()
                || matches!(record.command, Command::Ensure(_))
                    != matches!(stage, Stage::Create | Stage::CleanupCreation))
        {
            return Err(Error::InvalidTransition);
        }
        self.guard.before_write(WritePoint::Progress)?;
        let changed = self.connection.execute(
            "UPDATE executions SET state=?1,progress=?2 WHERE execution=?3 AND progress=?4",
            params![
                progress.kind(),
                serde_json::to_string(&progress)?,
                record.command.execution_id().as_str(),
                serde_json::to_string(&record.progress)?
            ],
        )?;
        if changed != 1 {
            return Err(Error::InvalidTransition);
        }
        Ok(Execution {
            progress,
            ..record.clone()
        })
    }

    /// Commits ownership facts, result and its original replay envelope in one transaction.
    pub fn complete(
        &mut self,
        record: &Execution,
        result: WorktreeExecutionResult,
    ) -> Result<(), Error> {
        if matches!(record.progress, Progress::Completed { .. }) {
            return Err(Error::InvalidTransition);
        }
        validate_result(&record.command, &result)?;
        if let WorktreeExecutionResult::Ready(ready) = &result {
            let target = record.target.as_ref().ok_or(Error::InvalidTransition)?;
            if ready.facts.branch != target.branch
                || ready.facts.base_commit != target.base_commit
                || Some(ready.facts.path.as_str()) != target.path.to_str()
            {
                return Err(Error::IdentityConflict);
            }
        }
        self.guard.before_write(WritePoint::Complete)?;
        let tx = self.connection.transaction()?;
        let changed = tx.execute("UPDATE executions SET state='completed',progress=?1 WHERE execution=?2 AND progress=?3", params![serde_json::to_string(&Progress::Completed {result: result.clone()})?, record.command.execution_id().as_str(), serde_json::to_string(&record.progress)?])?;
        if changed != 1 {
            return Err(Error::InvalidTransition);
        }
        let state = match &result {
            WorktreeExecutionResult::Ready(_) => Some(ResourceState::Present),
            WorktreeExecutionResult::Removed(_) => Some(ResourceState::Removed),
            // Definitive create failure has no unexplained effects; retire its reservation.
            WorktreeExecutionResult::Failed(_) => Some(ResourceState::Removed),
            WorktreeExecutionResult::RemovalFailed(_) => None,
        };
        if let Some(state) = state {
            let data: String = tx.query_row(
                "SELECT data FROM resources WHERE worktree=?1",
                [record.command.spec().worktree_id.as_str()],
                |row| row.get(/*idx*/ 0),
            )?;
            let mut resource: Resource = serde_json::from_str(&data)?;
            resource.state = state;
            tx.execute(
                "UPDATE resources SET data=?1,active=?2 WHERE worktree=?3",
                params![
                    serde_json::to_string(&resource)?,
                    i64::from(state != ResourceState::Removed),
                    record.command.spec().worktree_id.as_str()
                ],
            )?;
        }
        self.guard.before_write(WritePoint::Outbox)?;
        tx.execute(
            "INSERT INTO outbox VALUES (?1,?2)",
            params![
                record.command.execution_id().as_str(),
                serde_json::to_string(&record.command.event(result))?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Retains tombstones and reservations so deletion never grants ownership by itself.
    pub fn resource(&self, id: &WorktreeId) -> Result<Option<Resource>, Error> {
        let data: Option<String> = self
            .connection
            .query_row(
                "SELECT data FROM resources WHERE worktree=?1",
                [id.as_str()],
                |row| row.get(/*idx*/ 0),
            )
            .optional()?;
        data.map(|data| serde_json::from_str(&data).map_err(Error::from))
            .transpose()
    }

    /// Scans incomplete evidence; completed results remain available even if another record is unknown.
    pub fn recoverable(&self) -> Result<Vec<Execution>, Error> {
        let mut statement = self.connection.prepare(
            "SELECT input,target,progress FROM executions WHERE state!='completed' ORDER BY rowid",
        )?;
        let mut rows = statement.query([])?;
        let mut records = Vec::new();
        while let Some(row) = rows.next()? {
            records.push(decode(row)?);
        }
        Ok(records)
    }

    /// Enumerates exact original envelopes independently of status queries.
    pub fn pending_events(&self) -> Result<Vec<NodeToControllerMessage>, Error> {
        let mut statement = self
            .connection
            .prepare("SELECT event FROM outbox UNION ALL SELECT event FROM clone_outbox")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(/*idx*/ 0))?;
        rows.map(|data| Ok(serde_json::from_str(&data?)?)).collect()
    }

    /// Only the exact terminal event can be acknowledged; repeat acknowledgements are idempotent.
    pub fn acknowledge(&mut self, ack: &EventAckMessage) -> Result<(), Error> {
        ack.validate()?;
        if ack.payload.node_id != self.node_id {
            return Err(Error::NodeMismatch);
        }
        if self
            .identity_kind(&ack.operation_id, &ack.execution_id)?
            .as_deref()
            == Some("clone")
        {
            let record = self
                .find_clone(&ack.operation_id, &ack.execution_id)?
                .ok_or(Error::InvalidAck)?;
            if ack.sequence != Sequence::new(/*value*/ 1)
                || !matches!(record.progress, CloneProgress::Completed(_))
            {
                return Err(Error::InvalidAck);
            }
            self.guard.before_write(WritePoint::Acknowledge)?;
            self.connection.execute(
                "DELETE FROM clone_outbox WHERE execution=?1",
                [ack.execution_id.as_str()],
            )?;
            return Ok(());
        }
        let record = self
            .find(&ack.operation_id, &ack.execution_id)?
            .ok_or(Error::InvalidAck)?;
        if ack.sequence != Sequence::new(/*value*/ 1)
            || !matches!(record.progress, Progress::Completed { .. })
        {
            return Err(Error::InvalidAck);
        }
        self.guard.before_write(WritePoint::Acknowledge)?;
        self.connection.execute(
            "DELETE FROM outbox WHERE execution=?1",
            [ack.execution_id.as_str()],
        )?;
        Ok(())
    }
}

/// Ignores mutable base refs during deletion, but requires every ownership-bearing input to match.
pub fn owns(resource: &Resource, spec: &WorktreeExecutionSpec, target: &Target) -> bool {
    let mut expected = resource.spec.clone();
    expected.base_ref = spec.base_ref.clone();
    expected == *spec && resource.target == *target
}

/// Decodes complete records at the persistence seam without exposing raw SQL rows to callers.
fn decode(row: &rusqlite::Row<'_>) -> Result<Execution, Error> {
    let input: String = row.get(/*idx*/ 0)?;
    let target: Option<String> = row.get(/*idx*/ 1)?;
    let progress: String = row.get(/*idx*/ 2)?;
    Ok(Execution {
        command: serde_json::from_str(&input)?,
        target: target
            .map(|value| serde_json::from_str(&value))
            .transpose()?,
        progress: serde_json::from_str(&progress)?,
    })
}

/// Rejects results that could corrupt an execution's immutable ownership or operation kind.
fn validate_result(command: &Command, result: &WorktreeExecutionResult) -> Result<(), Error> {
    let (node, workspace, worktree, ensure) = match result {
        WorktreeExecutionResult::Ready(r) => (&r.node, &r.workspace_id, &r.worktree_id, true),
        WorktreeExecutionResult::Failed(r) => (&r.node, &r.workspace_id, &r.worktree_id, true),
        WorktreeExecutionResult::Removed(r) => (&r.node, &r.workspace_id, &r.worktree_id, false),
        WorktreeExecutionResult::RemovalFailed(r) => {
            (&r.node, &r.workspace_id, &r.worktree_id, false)
        }
    };
    if matches!(
        result,
        WorktreeExecutionResult::Failed(WorktreeFailed {
            failure: WorktreeFailure {
                code: WorktreeFailureCode::ResultUnknown,
                ..
            },
            ..
        }) | WorktreeExecutionResult::RemovalFailed(WorktreeRemovalFailed {
            failure: WorktreeFailure {
                code: WorktreeFailureCode::ResultUnknown,
                ..
            },
            ..
        })
    ) {
        return Err(Error::InvalidTransition);
    }
    let spec = command.spec();
    if node.node_id != spec.node_id
        || workspace != &spec.workspace_id
        || worktree != &spec.worktree_id
        || ensure != matches!(command, Command::Ensure(_))
    {
        return Err(Error::IdentityConflict);
    }
    command.event(result.clone()).validate()?;
    Ok(())
}
