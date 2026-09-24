//! Composite-region persistence: the iteration round and settlement transactions plus the
//! run-payload execution-state helpers they share with node completion.
//!
//! Every operation here runs in one immediate transaction so a round's terminal fact and the
//! following transition never split (ADR "iteration composite runtime" D2, D5).

use super::{engine_repository_error_from_database, insert_node_run, rewrite_current_nodes};
use ora_application::{
    AdvanceWorkflowRunResult, IterationRoundContinuation, NodeRunToStart, RepositoryError,
    RoundOutcome, WorkflowRunPayload, WorkflowVariablePool, WorkflowVariablePoolError,
};
use ora_domain::{WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

/// The composite operations of the engine repository, split out of the trait impl so the
/// scheduling-facing module stays within the size discipline. Each method opens its own
/// immediate transaction; the trait impl delegates here unchanged in semantics.
impl super::SqliteWorkflowRunEngineRepository {
    /// Starts one composite-region round atomically: binds the round's `item` and `index` pool
    /// variables and inserts the round's first node-run rows in one transaction, so a crash
    /// leaves either the whole round unstarted or the round variables consistent with its rows.
    pub(super) fn iteration_start_round(
        &self,
        run_id: &WorkflowRunId,
        owner_node_id: &str,
        round: u32,
        item: &serde_json::Value,
        node_runs: &[NodeRunToStart],
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                if !owner_row_is_running(&transaction, run_id, owner_node_id)? {
                    return Ok(AdvanceWorkflowRunResult::NotRunning);
                }
                let serialized_payload = run_payload_of(&transaction, run_id)?;
                let mut payload = parse_run_payload(serialized_payload.as_deref())?;
                write_declared_pool_variable(
                    &mut payload.variable_pool,
                    &format!("{owner_node_id}.item"),
                    owner_node_id,
                    item.clone(),
                )?;
                write_declared_pool_variable(
                    &mut payload.variable_pool,
                    &format!("{owner_node_id}.index"),
                    owner_node_id,
                    serde_json::json!(round),
                )?;
                persist_run_payload(&transaction, run_id.as_ref(), &payload)?;
                for node_run in node_runs {
                    insert_node_run(&transaction, run_id, node_run, now)?;
                }
                transaction.commit()?;
                Ok(AdvanceWorkflowRunResult::Advanced)
            })
            .map_err(engine_repository_error_from_database)
    }

    /// Settles one drained round and continues atomically: the ledger entry, the round's
    /// terminal fact, and the continuation (next round start, node completion, or node
    /// failure) commit in one transaction. Recording an already-settled round is a no-op so
    /// replanning after a crash stays idempotent.
    pub(super) fn iteration_settle_round(
        &self,
        run_id: &WorkflowRunId,
        owner_node_id: &str,
        round: u32,
        entry: RoundOutcome,
        continuation: IterationRoundContinuation,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction = Transaction::new(connection, TransactionBehavior::Immediate)?;
                if !owner_row_is_running(&transaction, run_id, owner_node_id)? {
                    return Ok(AdvanceWorkflowRunResult::NotRunning);
                }
                let serialized_payload = run_payload_of(&transaction, run_id)?;
                let mut payload = parse_run_payload(serialized_payload.as_deref())?;
                // Append-only ledger: an already-settled round means this settlement is a
                // replay whose continuation committed with it; treat it as a no-op.
                if payload
                    .record_round_outcome(owner_node_id, round, entry)
                    .is_err()
                {
                    return Ok(AdvanceWorkflowRunResult::NotRunning);
                }
                match continuation {
                    IterationRoundContinuation::StartNextRound {
                        round,
                        item,
                        node_runs,
                    } => {
                        write_declared_pool_variable(
                            &mut payload.variable_pool,
                            &format!("{owner_node_id}.item"),
                            owner_node_id,
                            item,
                        )?;
                        write_declared_pool_variable(
                            &mut payload.variable_pool,
                            &format!("{owner_node_id}.index"),
                            owner_node_id,
                            serde_json::json!(round),
                        )?;
                        persist_run_payload(&transaction, run_id.as_ref(), &payload)?;
                        for node_run in &node_runs {
                            insert_node_run(&transaction, run_id, node_run, now)?;
                        }
                    }
                    IterationRoundContinuation::Complete { exposed, output } => {
                        for (selector, value) in &exposed {
                            write_declared_pool_variable(
                                &mut payload.variable_pool,
                                selector,
                                owner_node_id,
                                value.clone(),
                            )?;
                        }
                        persist_run_payload(&transaction, run_id.as_ref(), &payload)?;
                        complete_owner_row(&transaction, run_id, owner_node_id, output, now)?;
                    }
                    IterationRoundContinuation::Fail { error } => {
                        persist_run_payload(&transaction, run_id.as_ref(), &payload)?;
                        fail_owner_row(&transaction, run_id, owner_node_id, &error, now)?;
                    }
                }
                transaction.commit()?;
                Ok(AdvanceWorkflowRunResult::Advanced)
            })
            .map_err(engine_repository_error_from_database)
    }

    /// Completes one composite node-run, writing its ledger-derived exposed variables through
    /// the pool's typed `set` and the node's display `output` in one transaction. Used for the
    /// empty-iterator-source path, where no round ever settles.
    pub(super) fn iteration_complete_node(
        &self,
        node_run_id: &WorkflowNodeRunId,
        owner_node_id: &str,
        exposed: &[(String, serde_json::Value)],
        output: Option<String>,
        now: i64,
    ) -> Result<AdvanceWorkflowRunResult, RepositoryError> {
        self.pool
            .with_connection_mut(|connection| {
                let transaction =
                    Transaction::new(connection, TransactionBehavior::Immediate)?;
                let Some((run_id, status)) = transaction
                    .query_row(
                        "SELECT run_id, status FROM workflow_node_runs WHERE id = ?1 AND is_deleted = 0",
                        params![node_run_id.as_ref()],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, i64>(1)?,
                            ))
                        },
                    )
                    .optional()?
                else {
                    return Ok(AdvanceWorkflowRunResult::NotFound);
                };
                if WorkflowNodeStatus::from_database_value(status)? != WorkflowNodeStatus::Running {
                    return Ok(AdvanceWorkflowRunResult::NotRunning);
                }
                let run_id = WorkflowRunId::new(run_id);
                let serialized_payload = run_payload_of(&transaction, &run_id)?;
                let mut payload = parse_run_payload(serialized_payload.as_deref())?;
                for (selector, value) in exposed {
                    write_declared_pool_variable(
                        &mut payload.variable_pool,
                        selector,
                        owner_node_id,
                        value.clone(),
                    )?;
                }
                persist_run_payload(&transaction, run_id.as_ref(), &payload)?;
                complete_owner_row(&transaction, &run_id, owner_node_id, output, now)?;
                transaction.commit()?;
                Ok(AdvanceWorkflowRunResult::Advanced)
            })
            .map_err(engine_repository_error_from_database)
    }
}

/// Commits public node values and private routing state with the node status transition.
///
/// Keeping Condition decisions outside the variable pool prevents scheduler implementation
/// details from becoming selectable workflow data while preserving restart-safe branch
/// projection. A Condition inside an iteration region records its decision per round, so a
/// later round can never overwrite an earlier round's branch (ADR "iteration composite
/// runtime" D5).
#[allow(clippy::too_many_arguments)]
pub(super) fn update_run_execution_state(
    transaction: &Transaction<'_>,
    run_id: &str,
    node_id: &str,
    node_type: &str,
    iteration: Option<u32>,
    output: Option<&str>,
    structured_output: Option<&serde_json::Value>,
    serialized_payload: Option<&str>,
) -> Result<(), crate::DatabaseError> {
    let Some(serialized_payload) = serialized_payload else {
        return Ok(());
    };
    let mut payload: WorkflowRunPayload = serde_json::from_str(serialized_payload)?;
    let mut changed = if node_type != "condition"
        && let Some(output) = output
    {
        write_pool_variable(
            &mut payload.variable_pool,
            &format!("{node_id}.output"),
            node_id,
            serde_json::Value::String(output.to_string()),
        )?
    } else {
        false
    };
    match node_type {
        "start" => {}
        "condition" => {
            if let Some(output) = output {
                match iteration {
                    Some(round) => {
                        let key = WorkflowRunPayload::iteration_decision_key(node_id, round);
                        changed |= payload
                            .iteration_condition_decisions
                            .get(&key)
                            .map(String::as_str)
                            != Some(output);
                        payload
                            .iteration_condition_decisions
                            .insert(key, output.to_string());
                    }
                    None => {
                        changed |= payload.condition_decisions.get(node_id).map(String::as_str)
                            != Some(output);
                        payload
                            .condition_decisions
                            .insert(node_id.to_string(), output.to_string());
                    }
                }
            }
        }
        "agent" => {
            if let Some(structured) = structured_output {
                changed |= write_pool_variable(
                    &mut payload.variable_pool,
                    &format!("{node_id}.structured_output"),
                    node_id,
                    structured.clone(),
                )?;
            }
        }
        _ => {}
    }
    if changed {
        persist_run_payload(transaction, run_id, &payload)?;
    }
    Ok(())
}

/// Writes one pool variable through its declared owner, reporting whether the pool changed.
pub(super) fn write_pool_variable(
    pool: &mut WorkflowVariablePool,
    selector: &str,
    writer: &str,
    value: serde_json::Value,
) -> Result<bool, rusqlite::Error> {
    if !pool.catalog.contains_key(selector) {
        return Ok(false);
    }
    pool.set(selector, writer, value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    Ok(true)
}

/// Whether the composite owner's own node-run row is still `Running` for the given run.
fn owner_row_is_running(
    transaction: &Transaction<'_>,
    run_id: &WorkflowRunId,
    owner_node_id: &str,
) -> Result<bool, crate::DatabaseError> {
    let status = transaction
        .query_row(
            "SELECT status FROM workflow_node_runs
             WHERE run_id = ?1 AND node_id = ?2 AND iteration IS NULL AND is_deleted = 0
             ORDER BY created_at DESC, id DESC LIMIT 1",
            params![run_id.as_ref(), owner_node_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    Ok(matches!(
        status,
        Some(status) if WorkflowNodeStatus::from_database_value(status)? == WorkflowNodeStatus::Running
    ))
}

/// Reads the run's serialized payload column.
fn run_payload_of(
    transaction: &Transaction<'_>,
    run_id: &WorkflowRunId,
) -> Result<Option<String>, crate::DatabaseError> {
    transaction
        .query_row(
            "SELECT payload FROM workflow_runs WHERE id = ?1 AND is_deleted = 0",
            params![run_id.as_ref()],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        .map_or(Ok(None), |payload| Ok(Some(payload)))
}

/// Parses the run payload, defaulting for runs persisted before typed payloads existed.
fn parse_run_payload(serialized: Option<&str>) -> Result<WorkflowRunPayload, crate::DatabaseError> {
    match serialized {
        Some(serialized) => Ok(serde_json::from_str(serialized)?),
        None => Ok(WorkflowRunPayload::default()),
    }
}

/// Persists the run payload inside the active transaction.
fn persist_run_payload(
    transaction: &Transaction<'_>,
    run_id: &str,
    payload: &WorkflowRunPayload,
) -> Result<(), crate::DatabaseError> {
    transaction.execute(
        "UPDATE workflow_runs SET payload = ?2 WHERE id = ?1 AND is_deleted = 0",
        params![run_id, serde_json::to_string(payload)?],
    )?;
    Ok(())
}

/// Completes the composite owner's latest outer row and releases it from the anchor.
fn complete_owner_row(
    transaction: &Transaction<'_>,
    run_id: &WorkflowRunId,
    owner_node_id: &str,
    output: Option<String>,
    now: i64,
) -> Result<(), crate::DatabaseError> {
    transaction.execute(
        "UPDATE workflow_node_runs SET status = ?3, output = ?4, finished_at = ?5, updated_at = ?5
         WHERE run_id = ?1 AND node_id = ?2 AND iteration IS NULL AND status = ?6 AND is_deleted = 0",
        params![
            run_id.as_ref(),
            owner_node_id,
            WorkflowNodeStatus::Succeeded.database_value(),
            output,
            now,
            WorkflowNodeStatus::Running.database_value(),
        ],
    )?;
    rewrite_current_nodes(transaction, run_id, now, |current_nodes| {
        current_nodes.retain(|id| id != owner_node_id);
    })?;
    Ok(())
}

/// Fails the composite owner's latest outer row together with its run (own failures always
/// propagate; ADR "iteration composite runtime" D6).
fn fail_owner_row(
    transaction: &Transaction<'_>,
    run_id: &WorkflowRunId,
    owner_node_id: &str,
    error: &str,
    now: i64,
) -> Result<(), crate::DatabaseError> {
    transaction.execute(
        "UPDATE workflow_node_runs SET status = ?3, error = ?4, finished_at = ?5, updated_at = ?5
         WHERE run_id = ?1 AND node_id = ?2 AND iteration IS NULL AND status = ?6 AND is_deleted = 0",
        params![
            run_id.as_ref(),
            owner_node_id,
            WorkflowNodeStatus::Failed.database_value(),
            error,
            now,
            WorkflowNodeStatus::Running.database_value(),
        ],
    )?;
    rewrite_current_nodes(transaction, run_id, now, |current_nodes| {
        current_nodes.clear();
        current_nodes.push(owner_node_id.to_string());
    })?;
    super::retry::settle_retry_waits(
        transaction,
        run_id.as_ref(),
        WorkflowNodeStatus::Cancelled,
        Some(super::retry::RETRY_ABANDONED),
        now,
    )?;
    transaction.execute(
        "UPDATE workflow_runs SET run_status = ?2, error = ?3, finished_at = ?4, updated_at = ?4
         WHERE id = ?1 AND is_deleted = 0",
        params![
            run_id.as_ref(),
            ora_domain::WorkflowRunStatus::Failed.database_value(),
            error,
            now,
        ],
    )?;
    Ok(())
}

/// Writes one declared pool variable strictly: an undeclared selector is a hard error instead
/// of the silent skip `write_pool_variable` uses for optional node outputs, because the
/// composite's round bindings and exposed variables are contract, not best-effort.
fn write_declared_pool_variable(
    pool: &mut WorkflowVariablePool,
    selector: &str,
    writer: &str,
    value: serde_json::Value,
) -> Result<bool, crate::DatabaseError> {
    if !pool.catalog.contains_key(selector) {
        return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(
            WorkflowVariablePoolError::Undeclared {
                selector: selector.to_string(),
            },
        ))
        .into());
    }
    pool.set(selector, writer, value)
        .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    Ok(true)
}
