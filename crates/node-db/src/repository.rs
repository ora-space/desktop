use crate::*;
use ora_node_protocol::*;
use rusqlite::{OptionalExtension, params};

impl<G: WriteGuard> NodeDatabase<G> {
    /// Resolves both identity keys before any business-specific lookup, including cross-capability conflicts.
    pub(crate) fn identity_kind(
        &self,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<Option<String>, Error> {
        let found: Option<(String, String, String)> = self.connection.query_row(
            "SELECT operation,execution,kind FROM execution_identities WHERE operation=?1 OR execution=?2",
            params![operation.as_str(), execution.as_str()],
            |r| Ok((r.get(/*idx*/ 0)?, r.get(/*idx*/ 1)?, r.get(/*idx*/ 2)?)),
        ).optional()?;
        match found {
            Some((op, id, kind)) if op == operation.as_str() && id == execution.as_str() => {
                Ok(Some(kind))
            }
            Some(_) => Err(Error::IdentityConflict),
            None => Ok(None),
        }
    }

    /// Reads original acquisition evidence without accessing a remote or changing a reservation.
    pub fn find_clone(
        &self,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<Option<CloneExecution>, Error> {
        if self
            .identity_kind(operation, execution)?
            .is_some_and(|kind| kind != "clone")
        {
            return Err(Error::IdentityConflict);
        }
        let data: Option<(String, String, String)> = self
            .connection
            .query_row(
                "SELECT input,target,progress FROM clone_executions WHERE execution=?1",
                [execution.as_str()],
                |r| Ok((r.get(/*idx*/ 0)?, r.get(/*idx*/ 1)?, r.get(/*idx*/ 2)?)),
            )
            .optional()?;
        data.map(decode).transpose()
    }

    /// Freezes a unique generated destination and full input atomically before filesystem work.
    pub fn accept_clone(
        &mut self,
        command: &CloneRepositoryMessage,
        target: &CloneTarget,
    ) -> Result<CloneExecution, Error> {
        command.validate()?;
        if command.payload.spec.node_id != self.node_id {
            return Err(Error::NodeMismatch);
        }
        if let Some(record) = self.find_clone(&command.operation_id, &command.execution_id)? {
            return if record.command == *command {
                Ok(record)
            } else {
                Err(Error::IdentityConflict)
            };
        }
        if target.repository_id.as_str().trim().is_empty()
            || !target.root.is_absolute()
            || target.path.parent() != Some(target.root.as_path())
            || target.path.file_name().is_none()
            || target.path.to_str().is_none()
        {
            return Err(Error::ResourceConflict);
        }
        let mut paths = self.connection.prepare(
            "SELECT path FROM resources WHERE active=1 UNION ALL SELECT path FROM clone_executions",
        )?;
        for path in paths.query_map([], |r| r.get::<_, String>(/*idx*/ 0))? {
            let path = std::path::PathBuf::from(path?);
            if path.starts_with(&target.path) || target.path.starts_with(&path) {
                return Err(Error::ResourceConflict);
            }
        }
        drop(paths);
        let progress = CloneProgress::Pending(ClonePhase::Reserved);
        self.guard.before_write(WritePoint::Accept)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "INSERT INTO execution_identities VALUES (?1,?2,'clone')",
            params![command.operation_id.as_str(), command.execution_id.as_str()],
        )?;
        tx.execute(
            "INSERT INTO clone_executions VALUES (?1,?2,?3,?4,?5,?6,'accepted',?7)",
            params![
                command.operation_id.as_str(),
                command.execution_id.as_str(),
                serde_json::to_string(command)?,
                target.repository_id.as_str(),
                target.path.to_str(),
                serde_json::to_string(target)?,
                serde_json::to_string(&progress)?,
            ],
        )?;
        tx.commit()?;
        Ok(CloneExecution {
            command: command.clone(),
            target: target.clone(),
            progress,
        })
    }

    /// Advances only along the original directory identity; stale or backwards evidence is refused.
    pub fn advance_clone(
        &mut self,
        record: &CloneExecution,
        progress: CloneProgress,
    ) -> Result<CloneExecution, Error> {
        let (
            CloneProgress::Pending(before) | CloneProgress::Unknown(before),
            CloneProgress::Pending(after) | CloneProgress::Unknown(after),
        ) = (&record.progress, &progress)
        else {
            return Err(Error::InvalidTransition);
        };
        let valid = match (before, after) {
            (ClonePhase::Reserved, ClonePhase::Reserved) => true,
            (ClonePhase::Reserved, ClonePhase::DirectoryCreated { identity }) => {
                !identity.is_empty()
            }
            (
                ClonePhase::DirectoryCreated { identity: a },
                ClonePhase::DirectoryCreated { identity: b }
                | ClonePhase::Dispatched { identity: b },
            )
            | (ClonePhase::Dispatched { identity: a }, ClonePhase::Dispatched { identity: b }) => {
                a == b && !a.is_empty()
            }
            (ClonePhase::Reserved, ClonePhase::Dispatched { .. })
            | (
                ClonePhase::DirectoryCreated { .. } | ClonePhase::Dispatched { .. },
                ClonePhase::Reserved,
            )
            | (ClonePhase::Dispatched { .. }, ClonePhase::DirectoryCreated { .. }) => false,
        };
        if !valid {
            return Err(Error::InvalidTransition);
        }
        self.guard.before_write(WritePoint::Progress)?;
        let changed = self.connection.execute(
            "UPDATE clone_executions SET state=?1,progress=?2 WHERE execution=?3 AND input=?4 AND target=?5 AND progress=?6",
            params![progress.kind(), serde_json::to_string(&progress)?, record.command.execution_id.as_str(),
                serde_json::to_string(&record.command)?, serde_json::to_string(&record.target)?, serde_json::to_string(&record.progress)?],
        )?;
        if changed != 1 {
            return Err(Error::InvalidTransition);
        }
        Ok(CloneExecution {
            progress,
            ..record.clone()
        })
    }

    /// Commits only attributable terminal evidence after all managed writers have been cleaned.
    pub fn complete_clone(
        &mut self,
        record: &CloneExecution,
        result: CloneExecutionResult,
    ) -> Result<(), Error> {
        record.event(result.clone()).validate()?;
        let phase = match &record.progress {
            CloneProgress::Pending(phase) | CloneProgress::Unknown(phase) => phase,
            CloneProgress::Completed(_) => return Err(Error::InvalidTransition),
        };
        let (spec, destination) = match &result {
            CloneExecutionResult::CloneReady(ready) => {
                (&ready.spec, Some((&ready.repository_id, &ready.path)))
            }
            CloneExecutionResult::CloneFailed(failed) => (
                &failed.spec,
                match &failed.residual {
                    CloneResidual::NoDirectory {} => None,
                    CloneResidual::Retained {
                        repository_id,
                        path,
                    } => Some((repository_id, path)),
                },
            ),
        };
        if spec != &record.command.payload.spec || spec.node_id != self.node_id {
            return Err(Error::IdentityConflict);
        }
        if let Some((id, path)) = destination {
            if id != &record.target.repository_id
                || Some(path.as_str()) != record.target.path.to_str()
                || matches!(phase, ClonePhase::Reserved)
            {
                return Err(Error::IdentityConflict);
            }
        } else if !matches!(phase, ClonePhase::Reserved) {
            return Err(Error::InvalidTransition);
        }
        let pending: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM process_attempts WHERE execution=?1 AND cleaned=0)",
            [record.command.execution_id.as_str()],
            |r| r.get(/*idx*/ 0),
        )?;
        if pending {
            return Err(Error::InvalidTransition);
        }
        if matches!(phase, ClonePhase::Dispatched { .. }) {
            let codes: Vec<i32> = self.connection.prepare(
                "SELECT exit_code FROM process_outcomes JOIN process_attempts USING(run) WHERE execution=?1",
            )?.query_map([record.command.execution_id.as_str()], |r| r.get(/*idx*/ 0))?.collect::<Result<_, _>>()?;
            let success = matches!(result, CloneExecutionResult::CloneReady(_));
            if codes.len() != 1 || (success && codes[0] != 0) {
                return Err(Error::InvalidTransition);
            }
        } else if matches!(result, CloneExecutionResult::CloneReady(_)) {
            return Err(Error::InvalidTransition);
        }
        self.guard.before_write(WritePoint::Complete)?;
        let tx = self.connection.transaction()?;
        let changed = tx.execute(
            "UPDATE clone_executions SET state='completed',progress=?1 WHERE execution=?2 AND input=?3 AND target=?4 AND progress=?5",
            params![serde_json::to_string(&CloneProgress::Completed(result.clone()))?, record.command.execution_id.as_str(),
                serde_json::to_string(&record.command)?, serde_json::to_string(&record.target)?, serde_json::to_string(&record.progress)?],
        )?;
        if changed != 1 {
            return Err(Error::InvalidTransition);
        }
        self.guard.before_write(WritePoint::Outbox)?;
        tx.execute(
            "INSERT INTO clone_outbox VALUES (?1,?2)",
            params![
                record.command.execution_id.as_str(),
                serde_json::to_string(&record.event(result))?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Keeps every failed/unknown destination reserved; only unfinished execution evidence is reconciled.
    pub fn recoverable_clones(&self) -> Result<Vec<CloneExecution>, Error> {
        self.connection.prepare("SELECT input,target,progress FROM clone_executions WHERE state!='completed' ORDER BY rowid")?
            .query_map([], |r| Ok((r.get(/*idx*/ 0)?, r.get(/*idx*/ 1)?, r.get(/*idx*/ 2)?)))?
            .map(|row| decode(row?)).collect()
    }

    /// Reads status without guessing a capability or acknowledging its pending event.
    pub fn execution_state(
        &self,
        operation: &OperationId,
        execution: &ExecutionId,
    ) -> Result<ExecutionState, Error> {
        match self.identity_kind(operation, execution)?.as_deref() {
            Some("worktree") => Ok(self
                .find(operation, execution)?
                .map_or(ExecutionState::Unknown, |r| r.progress.state())),
            Some("clone") => Ok(self
                .find_clone(operation, execution)?
                .map_or(ExecutionState::Unknown, |r| r.progress.state())),
            None => Ok(ExecutionState::Unknown),
            Some(_) => Err(Error::InvalidSchema),
        }
    }
}

/// Decodes clone-owned state without rewriting historical input or directory evidence.
fn decode((input, target, progress): (String, String, String)) -> Result<CloneExecution, Error> {
    Ok(CloneExecution {
        command: serde_json::from_str(&input)?,
        target: serde_json::from_str(&target)?,
        progress: serde_json::from_str(&progress)?,
    })
}
