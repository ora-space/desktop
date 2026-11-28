//! Run-payload state maintenance: Start-input updates, system-variable seeds, the run-input
//! mirror, and the restart reset.
//!
//! These helpers mutate the private `WorkflowRunPayload` JSON behind the run's state
//! transitions; they live here so the scheduling-facing repository module stays within the
//! module-size discipline.

use super::iteration::write_pool_variable;
use ora_application::{RunInputRejectionReason, WorkflowRunPayload, WorkflowVariablePool};
use ora_domain::WorkflowRunId;
use rusqlite::{Transaction, params};
use thiserror::Error;

/// Failures raised while folding user-supplied Start values into a run payload.
#[derive(Debug, Error)]
pub(super) enum UpdateTaskInputError {
    /// One value was refused by the run's own declaration; surfaced to the user with its
    /// variable name instead of an internal repository failure.
    #[error("Start variable {variable} was rejected: {}", .reason.detail())]
    Rejected {
        variable: String,
        reason: RunInputRejectionReason,
    },
    /// The stored payload could not be read or written.
    #[error(transparent)]
    Database(#[from] crate::DatabaseError),
}

pub(super) fn seed_system_variables(
    pool: &mut WorkflowVariablePool,
    workflow_id: &str,
    now: i64,
) -> Result<bool, rusqlite::Error> {
    let mut changed = false;
    changed |= write_pool_variable(
        pool,
        "sys.workflow_id",
        "sys",
        serde_json::Value::String(workflow_id.to_string()),
    )?;
    changed |= write_pool_variable(
        pool,
        "sys.timestamp",
        "sys",
        serde_json::Value::Number(now.into()),
    )?;
    Ok(changed)
}

/// Folds user-supplied Start values into a run payload.
///
/// Single-rejection contract: values fold in `BTreeMap` order (alphabetical by variable name,
/// not form order), and the first rejected value aborts the whole submission before any value
/// is applied, so one save reports exactly one variable and the run keeps its previously stored
/// input. Surfacing every rejected field at once would need a multi-rejection port and public
/// error shape; until then callers must not assume batch diagnostics here.
pub(super) fn update_task_input_in_payload(
    serialized_payload: Option<&str>,
    variables: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Result<Option<String>, UpdateTaskInputError> {
    let Some(serialized_payload) = serialized_payload else {
        return Ok(None);
    };
    let mut payload: WorkflowRunPayload =
        serde_json::from_str(serialized_payload).map_err(|error| {
            UpdateTaskInputError::Database(crate::DatabaseError::CorruptWorkflowRunState(error))
        })?;
    let start_writer = resolve_start_writer(&payload);
    remove_legacy_instruction_aliases(&mut payload, start_writer.as_deref());
    if let Some(start_writer) = start_writer.as_ref() {
        for (name, value) in variables {
            let selector = format!("{start_writer}.{name}");
            let definition = payload
                .variable_pool
                .catalog
                .get(&selector)
                .ok_or_else(|| UpdateTaskInputError::Rejected {
                    variable: name.clone(),
                    reason: RunInputRejectionReason::Undeclared,
                })?;
            if &definition.writer != start_writer {
                return Err(UpdateTaskInputError::Rejected {
                    variable: name.clone(),
                    reason: RunInputRejectionReason::NotStartOwned,
                });
            }
            if value.is_null() {
                payload.variable_pool.values.remove(&selector);
            } else {
                payload
                    .variable_pool
                    .set(&selector, start_writer, value.clone())
                    .map_err(|error| match error {
                        ora_application::WorkflowVariablePoolError::TypeMismatch {
                            value_type,
                            ..
                        } => UpdateTaskInputError::Rejected {
                            variable: name.clone(),
                            reason: RunInputRejectionReason::TypeMismatch {
                                expected_type: value_type,
                            },
                        },
                        ora_application::WorkflowVariablePoolError::LengthExceeded {
                            max_length,
                            ..
                        } => UpdateTaskInputError::Rejected {
                            variable: name.clone(),
                            reason: RunInputRejectionReason::LengthExceeded { max_length },
                        },
                        // Defensive guard: `set()` re-checks the declaration and writer the
                        // pre-checks above already validated, so reaching here means the stored
                        // pool disagrees with its own catalog. Kept as a dedicated corrupt-state
                        // variant so it never surfaces as a misleading SQL conversion failure.
                        other => UpdateTaskInputError::Database(
                            crate::DatabaseError::CorruptWorkflowVariablePool(other),
                        ),
                    })?;
            }
        }
    } else if !variables.is_empty() {
        return Err(UpdateTaskInputError::Database(
            rusqlite::Error::InvalidParameterName(
                "run payload has no Start node owner".to_string(),
            )
            .into(),
        ));
    }
    payload.variable_pool.revision = payload.variable_pool.revision.saturating_add(1);
    Ok(Some(serde_json::to_string(&payload).map_err(|error| {
        UpdateTaskInputError::Database(crate::DatabaseError::CorruptWorkflowRunState(error))
    })?))
}

pub(super) fn mirror_run_input_into_pool(
    serialized_payload: Option<&str>,
    input: Option<&str>,
) -> Result<Option<String>, crate::DatabaseError> {
    let Some(serialized_payload) = serialized_payload else {
        return Ok(None);
    };
    let mut payload: WorkflowRunPayload = serde_json::from_str(serialized_payload)?;
    let Some(start_writer) = resolve_start_writer(&payload) else {
        return Ok(Some(serialized_payload.to_string()));
    };
    let selector = format!("{start_writer}.input");
    if !payload.variable_pool.catalog.contains_key(&selector) {
        return Ok(Some(serialized_payload.to_string()));
    }
    let changed = match input {
        Some(text) => payload
            .variable_pool
            .set(
                &selector,
                &start_writer,
                serde_json::Value::String(text.to_string()),
            )
            .map(|()| true)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?,
        None => payload.variable_pool.values.remove(&selector).is_some(),
    };
    if changed {
        payload.variable_pool.revision = payload.variable_pool.revision.saturating_add(1);
    }
    Ok(Some(serde_json::to_string(&payload)?))
}

pub(super) fn reset_run_execution_state(
    transaction: &Transaction<'_>,
    run_id: &WorkflowRunId,
    serialized_payload: Option<&str>,
) -> Result<(), crate::DatabaseError> {
    let Some(serialized_payload) = serialized_payload else {
        return Ok(());
    };
    let mut payload: WorkflowRunPayload = serde_json::from_str(serialized_payload)?;
    if payload.variable_pool.catalog.is_empty()
        && payload.condition_decisions.is_empty()
        && payload.debug_parked_withdrawals.is_empty()
    {
        return Ok(());
    }
    let start_writer = resolve_start_writer(&payload);
    remove_legacy_instruction_aliases(&mut payload, start_writer.as_deref());
    let pool = &mut payload.variable_pool;
    // Explicit deployment variables survive restart; values produced during execution do not.
    pool.values.retain(|selector, _| {
        start_writer.as_ref().is_some_and(|writer| {
            pool.catalog
                .get(selector)
                .is_some_and(|definition| &definition.writer == writer)
        })
    });
    pool.revision = pool.revision.saturating_add(1);
    payload.condition_decisions.clear();
    // A restart is a fresh execution: the per-run iteration ledger and per-round branch
    // decisions reset with it (ADR "iteration composite runtime" D5).
    payload.iteration_ledger.clear();
    payload.iteration_condition_decisions.clear();
    // Withdrawals parked by a debug edit on the old root are moot: every value they named is
    // gone, and keeping them would retire the new root's results on a later recovery.
    payload.debug_parked_withdrawals.clear();
    transaction.execute(
        "UPDATE workflow_runs SET payload = ?2 WHERE id = ?1 AND is_deleted = 0",
        params![run_id.as_ref(), serde_json::to_string(&payload)?],
    )?;
    Ok(())
}

pub(super) fn resolve_start_writer(payload: &WorkflowRunPayload) -> Option<String> {
    payload.start_node_id.clone().or_else(|| {
        payload
            .variable_pool
            .catalog
            .iter()
            .find_map(|(selector, definition)| {
                selector
                    .ends_with(".request")
                    .then(|| definition.writer.clone())
            })
    })
}

pub(super) fn remove_legacy_instruction_aliases(
    payload: &mut WorkflowRunPayload,
    start_writer: Option<&str>,
) {
    payload.variable_pool.catalog.remove("sys.task");
    payload.variable_pool.values.remove("sys.task");
    if let Some(start_writer) = start_writer {
        let request_selector = format!("{start_writer}.request");
        payload.variable_pool.catalog.remove(&request_selector);
        payload.variable_pool.values.remove(&request_selector);
    }
}
