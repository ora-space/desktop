//! Resolve the whole fixed inventory; no latest-session lookup is permitted.
use super::*;

/// A durable frozen plan needs the writer fence again, but never the original history files.
pub(super) fn writers_stopped(
    node: &ManagedNode,
    agents: Option<&SessionHost>,
    record: &RevisionDelivery,
) -> Result<bool, crate::Error> {
    let spec = &record.command.payload.spec;
    let Some(workflow) = &spec.workflow else {
        return Ok(false);
    };
    let journal = node.database.session_journal()?;
    for attempt in &workflow.sessions {
        let Some(session) = node.database.delivery_session(&attempt.execution_id)? else {
            return Ok(false);
        };
        if !matches!(session.state, ExecutionState::Completed(_))
            || session.command.payload.spec.checkout_execution_id != spec.checkout_execution_id
            || session.command.payload.spec.prior_revision.is_some()
            || journal.restore_phase(&attempt.execution_id)?
                == Some(ora_node_db::RestorePhase::LegacyUnknown)
            || !journal
                .pending_restore_attempts(&attempt.execution_id)?
                .is_empty()
            || agents.is_some_and(|agents| !agents.writers_settled(&attempt.execution_id))
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Every attempt must be terminal on this Node and use the one frozen checkout.
pub(super) fn inputs(
    node: &ManagedNode,
    agents: Option<&SessionHost>,
    record: &RevisionDelivery,
) -> Result<Inputs, crate::Error> {
    let spec = &record.command.payload.spec;
    let Some(workflow) = &spec.workflow else {
        return Ok(Inputs::Failed(RevisionFailureCode::SessionNotSettled));
    };
    let Some(checkout) = checkout_of(node, spec)? else {
        return Ok(Inputs::Failed(RevisionFailureCode::CheckoutUnavailable));
    };
    let journal = node.database.session_journal()?;
    let mut sources = Vec::new();
    let mut author = None;
    for attempt in &workflow.sessions {
        let Some(session) = node.database.delivery_session(&attempt.execution_id)? else {
            // A registered execution absent from the ledger is unknown, never not_started.
            return Ok(Inputs::Failed(RevisionFailureCode::SessionNotSettled));
        };
        let ExecutionState::Completed(ExecutionResult::AgentSession(_)) = session.state else {
            return Ok(Inputs::Wait);
        };
        if session.command.payload.spec.checkout_execution_id != spec.checkout_execution_id
            || session.command.payload.spec.prior_revision.is_some()
        {
            return Ok(Inputs::Failed(RevisionFailureCode::CheckoutUnavailable));
        }
        if journal.restore_phase(&attempt.execution_id)?
            == Some(ora_node_db::RestorePhase::LegacyUnknown)
            || !journal
                .pending_restore_attempts(&attempt.execution_id)?
                .is_empty()
        {
            return Ok(Inputs::Wait);
        }
        let admitted = journal.history_admitted(&attempt.execution_id)?;
        if !admitted {
            // This durable absence is valid only for new executions: the upgrade marks every
            // pre-existing execution as uncertain. It never infers not_started from missing bytes.
            author.get_or_insert(session.command.payload.spec.git_identity);
            sources.push((attempt.clone(), None));
            continue;
        }
        if let Some(agents) = agents
            && agents.sealed_history(&attempt.execution_id).is_err()
        {
            return Ok(Inputs::Wait);
        }
        let path = ora_history::history_path(
            &node.home_directory().join("sessions"),
            attempt.execution_id.as_str(),
        );
        let Ok(path) = path else {
            return Ok(Inputs::Failed(RevisionFailureCode::HistoryUnavailable));
        };
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            return Ok(Inputs::Failed(RevisionFailureCode::HistoryUnavailable));
        };
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Ok(Inputs::Failed(RevisionFailureCode::HistoryUnavailable));
        }
        if metadata.len() == 0 {
            return Ok(Inputs::Failed(RevisionFailureCode::HistoryUnavailable));
        }
        author.get_or_insert(session.command.payload.spec.git_identity);
        sources.push((attempt.clone(), Some(path)));
    }
    let Some(author) = author else {
        return Ok(Inputs::Failed(RevisionFailureCode::HistoryUnavailable));
    };
    Ok(Inputs::Ready(Box::new(Preparation {
        execution: record.command.execution_id.clone(),
        mode: SnapshotMode::Worktree,
        spec: spec.clone(),
        checkout,
        history: HistoryInput::Workflow(sources),
        author,
        node: node.identity().clone(),
        owner: node.git.runner().process_config().workload_uid,
    })))
}
