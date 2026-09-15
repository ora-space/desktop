//! Best-effort recovery of workflow runs and their owned baseline side files.

use super::engine::{ConcreteWorkflowRunEngine, reconcile_running_workflow_runs};
use crate::clock::SystemClock;
use crate::git_cleanup::KeyedResourceLocks;
use ora_application::{
    Clock, CompositeRegion, NodeType, RepositoryError, WorkflowGraph, WorkflowRunEngineRepository,
};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_domain::{WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId};
use ora_logging::ora_error;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

/// Fails runs interrupted by a previous process, then reconciles the survivors.
///
/// Runs that were `Running` or `Failed` when the process died have their interrupted node runs
/// marked `Failed` with `interrupted_by_restart`. The sweep is graph-aware for composite
/// regions (ADR "iteration composite runtime" D2): a `Running` iteration row is handed back to
/// its runtime instead of being failed — its interrupted round rows fail, the run survives, and
/// the runtime settles the interrupted round as a failed round on the next advance. Surviving
/// `Running` runs are then reconciled: stalled ones resume scheduling, and invalid `Pending`
/// nodes fail closed. The sweep is idempotent and best-effort so a storage failure cannot block
/// startup.
pub(crate) fn run_workflow_run_boot_sweep(
    pool: &RepositoryPool,
    engine: &Arc<ConcreteWorkflowRunEngine>,
    run_locks: &Arc<KeyedResourceLocks>,
    clock: SystemClock,
) {
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let run_ids = match repository.list_recoverable_runs() {
        Ok(run_ids) => run_ids,
        Err(error) => {
            ora_error!(error = %error, "workflow run boot sweep failed to list recoverable runs");
            return;
        }
    };
    for run_id in &run_ids {
        if let Err(error) = sweep_one_run(&repository, run_id, clock.now_timestamp_millis()) {
            ora_error!(run_id = %run_id, error = %error, "workflow run boot sweep failed to sweep one run");
        }
    }
    reconcile_running_workflow_runs(engine, run_locks, pool);
}

/// The crash-sweep decision for one recoverable run.
#[derive(Debug, PartialEq, Eq)]
enum SweepDecision {
    /// Nothing to do: the run has no `Running` row (parked awaiting input or terminal).
    Skip,
    /// The interrupted rows all sit inside the region of a still-`Running` composite node: they
    /// fail as interrupted while the composite row and the run survive for the runtime to
    /// settle the round (ADR "iteration composite runtime" D2).
    AbsorbIntoComposite(Vec<WorkflowNodeRunId>),
    /// Fail everything non-terminal together with the run (the pre-composite behavior).
    FailRun,
}

/// Applies the crash sweep to one recoverable run.
fn sweep_one_run(
    repository: &SqliteWorkflowRunEngineRepository,
    run_id: &WorkflowRunId,
    now: i64,
) -> Result<(), RepositoryError> {
    match decide_sweep(repository, run_id)? {
        SweepDecision::Skip => Ok(()),
        SweepDecision::AbsorbIntoComposite(interrupted) => {
            repository.fail_interrupted_node_runs(run_id, &interrupted, now)
        }
        SweepDecision::FailRun => {
            repository.fail_orphaned_node_runs(std::slice::from_ref(run_id), now)
        }
    }
}

/// Decides how one recoverable run survives the crash, by persisted rows plus graph structure.
fn decide_sweep(
    repository: &SqliteWorkflowRunEngineRepository,
    run_id: &WorkflowRunId,
) -> Result<SweepDecision, RepositoryError> {
    let node_runs = repository.list_node_runs(run_id)?;
    let running: Vec<&WorkflowNodeRun> = node_runs
        .iter()
        .filter(|node_run| node_run.status == WorkflowNodeStatus::Running)
        .collect();
    // No `Running` row means the run was parked awaiting input (or already terminal): the
    // human-owned pause survives the restart untouched.
    if running.is_empty() {
        return Ok(SweepDecision::Skip);
    }
    let context = repository.find_execution_context(run_id)?;
    let graph = context
        .as_ref()
        .and_then(|context| WorkflowGraph::parse(&context.graph_json).ok());
    // Without a parseable graph, membership cannot be proven; stay conservative and fail the
    // run exactly like the pre-composite sweep.
    let Some(graph) = graph else {
        return Ok(SweepDecision::FailRun);
    };

    let running_composites: Vec<&CompositeRegion> = running
        .iter()
        .filter(|node_run| is_composite_row(node_run))
        .filter_map(|node_run| graph.region(&node_run.node_id))
        .collect();
    if running_composites.is_empty() {
        return Ok(SweepDecision::FailRun);
    }
    // Every non-composite `Running` row must sit inside the region of a still-`Running`
    // composite node; an interrupted outer row fails the run (Run propagation).
    let mut interrupted = Vec::new();
    for node_run in &running {
        if is_composite_row(node_run) {
            continue;
        }
        let inside_running_composite = graph.region_owner(&node_run.node_id).is_some_and(|owner| {
            running_composites
                .iter()
                .any(|region| region.owner_id == owner)
        });
        if !inside_running_composite {
            return Ok(SweepDecision::FailRun);
        }
        interrupted.push(node_run.id.clone());
    }
    Ok(SweepDecision::AbsorbIntoComposite(interrupted))
}

/// Whether one row belongs to a composite runtime (the row's node resolves to a composite kind).
fn is_composite_row(node_run: &WorkflowNodeRun) -> bool {
    NodeType::from_str(&node_run.node_type)
        .map(NodeType::is_composite)
        .unwrap_or(false)
}

/// Deletes worktree-baseline side files whose node run is missing or no longer awaiting input.
///
/// Baselines exist only while an interactive node awaits input; a crash between a node's terminal
/// commit and its baseline deletion, or a node that failed without cleanup, leaves orphaned side
/// files that this sweep reclaims at the next boot.
pub(crate) fn prune_orphaned_baselines(pool: &RepositoryPool, baselines_root: &Path) {
    let Ok(entries) = std::fs::read_dir(baselines_root) else {
        return;
    };
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let node_run_id = ora_domain::WorkflowNodeRunId::new(name);
        let still_awaiting = repository
            .find_node_run_by_id(&node_run_id)
            .map(|node_run| {
                node_run.is_some_and(|node| node.status == ora_domain::WorkflowNodeStatus::Pending)
            })
            .unwrap_or(false);
        if !still_awaiting {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::run::test_fixture::{bootstrap, seeded_pending_run};
    use ora_application::WorkflowRunEngine;
    use ora_db::SqliteWorkflowRunEngineRepository;
    use ora_domain::WorkflowNodeStatus;
    use pretty_assertions::assert_eq;

    /// A run whose only interrupted rows sit inside a running iteration's region keeps the run
    /// and the composite row alive: the interrupted round settles as a failed ledger entry under
    /// `continue`, the remaining round still executes, and the node completes with the ledger
    /// projection (ADR "iteration composite runtime" D2, D4).
    #[test]
    fn a_crashed_round_inside_a_running_iteration_is_absorbed_not_failed() {
        crate::workflow::run::test_fixture::run_test(async {
            let (temp, pool) = bootstrap();
            let graph = r#"{"nodes":[
                {"id":"start","data":{"kind":"start","inputVariables":[{"name":"prs","valueType":"array[object]"}]}},
                {"id":"iter","data":{"kind":"iteration","iterationConfig":{
                    "iteratorSelector":["start","prs"],"collectSelector":["fix","output"],"errorStrategy":"continue","maxIterations":5}}},
                {"id":"fix","parentId":"iter","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"open_code","modelId":"m"},"prompt":"fix"}}},
                {"id":"out","data":{"kind":"output"}}
            ],"edges":[
                {"source":"start","target":"iter"},
                {"source":"iter","target":"fix"},
                {"source":"iter","target":"out"}
            ]}"#;
            let run_id = seeded_pending_run_with_source(&temp, &pool, graph);
            let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
            let engine = Arc::new(WorkflowRunEngine::new(
                repository.clone(),
                crate::workflow::run::test_fixture::NoopExecutor,
                ora_application::UuidWorkflowNodeRunIdGenerator::new(),
                crate::clock::SystemClock,
            ));
            engine.start(&run_id).unwrap();

            // Round 0 completed before the crash; round 1's agent was still running.
            let round_zero = repository
                .list_node_runs(&run_id)
                .unwrap()
                .into_iter()
                .find(|row| row.node_id == "fix" && row.iteration == Some(0))
                .expect("round 0 row");
            engine
                .complete_node(
                    &run_id,
                    &round_zero.id,
                    Some("fixed round zero".to_string()),
                    None,
                    None,
                    Vec::new(),
                )
                .unwrap();
            // The process "crashed" — run the real boot sweep and reconcile path.
            let locks = crate::workflow::run::test_fixture::locks().0;
            sweep_one_run(&repository, &run_id, 50).unwrap();
            crate::workflow::run::engine::reconcile_running_workflow_runs(&engine, &locks, &pool);

            // The sweep failed only the interrupted round row; the composite row and the run
            // survived, and reconcile drove them to the settled outcome: the interrupted
            // round became a failed ledger entry, the completed round was never re-run, and
            // the node finished with the ledger projection.
            let context = repository
                .find_execution_context(&run_id)
                .unwrap()
                .expect("run context");
            assert_eq!(context.run.status, ora_domain::WorkflowRunStatus::Succeeded);
            let node_runs = repository.list_node_runs(&run_id).unwrap();
            let fix_rounds: Vec<(Option<u32>, WorkflowNodeStatus)> = node_runs
                .iter()
                .filter(|row| row.node_id == "fix")
                .map(|row| (row.iteration, row.status))
                .collect();
            assert_eq!(
                fix_rounds,
                vec![
                    (Some(0), WorkflowNodeStatus::Succeeded),
                    (Some(1), WorkflowNodeStatus::Failed),
                ]
            );
            let iter_row = node_runs
                .iter()
                .find(|row| row.node_id == "iter")
                .expect("iteration row");
            assert_eq!(iter_row.status, WorkflowNodeStatus::Succeeded);

            let payload: ora_application::WorkflowRunPayload =
                serde_json::from_str(context.run.payload.as_deref().unwrap()).unwrap();
            assert_eq!(
                payload.variable_pool.values.get("iter.output").unwrap(),
                &serde_json::json!(["fixed round zero"])
            );
            assert_eq!(
                payload
                    .variable_pool
                    .values
                    .get("iter.failed_count")
                    .unwrap(),
                &serde_json::json!(1)
            );
            let entries = payload
                .variable_pool
                .values
                .get("iter.entries")
                .unwrap()
                .as_array()
                .unwrap();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0]["status"], serde_json::json!("succeeded"));
            assert_eq!(entries[1]["status"], serde_json::json!("failed"));
        });
    }

    /// A run with an interrupted outer agent row fails whole, like the pre-composite sweep.
    #[test]
    fn an_interrupted_outer_row_still_fails_the_run() {
        crate::workflow::run::test_fixture::run_test(async {
            let (temp, pool) = bootstrap();
            let run_id = seeded_pending_run(
                &temp,
                &pool,
                crate::workflow::run::test_fixture::TWO_AGENT_GRAPH,
            );
            let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
            let engine = WorkflowRunEngine::new(
                repository.clone(),
                crate::workflow::run::test_fixture::NoopExecutor,
                crate::workflow::run::test_fixture::SeqGen::default(),
                crate::workflow::run::test_fixture::ClockAt(40),
            );
            engine.start(&run_id).unwrap();
            sweep_one_run(&repository, &run_id, 50).unwrap();
            let context = repository.find_execution_context(&run_id).unwrap().unwrap();
            assert_eq!(context.run.status, ora_domain::WorkflowRunStatus::Failed);
            let node_runs = repository.list_node_runs(&run_id).unwrap();
            assert!(
                node_runs
                    .iter()
                    .filter(|row| row.status == WorkflowNodeStatus::Running)
                    .next()
                    .is_none()
            );
        });
    }

    /// Seeds a run whose Start variable `prs` carries a two-element array source.
    fn seeded_pending_run_with_source(
        temp: &tempfile::TempDir,
        pool: &ora_db::RepositoryPool,
        graph: &str,
    ) -> WorkflowRunId {
        let run_id = seeded_pending_run(temp, pool, graph);
        let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
        let mut variables = std::collections::BTreeMap::new();
        variables.insert(
            "prs".to_string(),
            serde_json::json!([{ "id": 1 }, { "id": 2 }]),
        );
        repository
            .update_run_input(&run_id, Some("kickoff".to_string()), variables, 35)
            .unwrap();
        run_id
    }
}
