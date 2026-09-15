//! The iteration composite runtime: pure advance planning over persisted facts.
//!
//! The runtime owns no state. Every scheduling wave hands the Running iteration node-run back
//! here with the graph, the committed node-run rows, and the run payload; the runtime answers
//! with the next transition ([`CompositeAdvancePlan`]) and the engine executes it through
//! repository transactions (ADR "iteration composite runtime" D2 advance, D3–D6).

use super::{CompositeAdvancePlan, CompositeContinuation, CompositeNodeRuntime, NodeRuntime};
use crate::workflow_run::engine::branch_projection::BranchProjection;
use crate::workflow_run::engine::graph::{WorkflowGraph, WorkflowGraphNode};
use crate::workflow_run::engine::iteration::{
    IterationConfig, IterationErrorStrategy, IterationLedger, RoundOutcome, project_exposed_values,
};
use crate::workflow_run::engine::ports::ExecutionContext;
use crate::workflow_run::engine::skill_delivery::WorkflowRunPayload;
use ora_domain::{WorkflowNodeRun, WorkflowNodeStatus};
use serde_json::Value;

/// The iteration runtime: foreach semantics over one array variable.
pub(super) struct IterationRuntime;

impl NodeRuntime for IterationRuntime {
    /// An iteration node-run records its iterator source selector as the scalar input.
    fn start_input(&self, node: &WorkflowGraphNode, _context: &ExecutionContext) -> Option<String> {
        node.iteration_config
            .as_ref()
            .map(|config| config.iterator_selector.qualified())
    }

    /// A completed iteration contributes its collected array as the run-output fallback, at the
    /// same precedence as a completed Agent and below a terminal Output.
    fn run_output_rank(&self) -> Option<u32> {
        Some(1)
    }
}

impl CompositeNodeRuntime for IterationRuntime {
    fn plan_advance(
        &self,
        node: &WorkflowGraphNode,
        graph: &WorkflowGraph,
        node_runs: &[WorkflowNodeRun],
        payload: &WorkflowRunPayload,
    ) -> Result<CompositeAdvancePlan, String> {
        let config = node
            .iteration_config
            .as_ref()
            .ok_or_else(|| format!("iteration node {} has no iteration config", node.id))?;
        let region = graph
            .region(&node.id)
            .ok_or_else(|| format!("iteration node {} has no region", node.id))?;
        let region_rows: Vec<&WorkflowNodeRun> = node_runs
            .iter()
            .filter(|row| row.iteration.is_some() && region.contains(&row.node_id))
            .collect();
        let ledger: IterationLedger = payload
            .iteration_ledger(&node.id)
            .cloned()
            .unwrap_or_default();

        // Startup boundary: no round has started and none has settled. The iterator source is
        // resolved, the safety ceiling is enforced before any round runs, and an empty source
        // completes the node immediately with empty exposed variables (ADR D1, D2).
        if region_rows.is_empty() && ledger.is_empty() {
            return plan_startup(node, config, graph, region, node_runs, payload);
        }

        let source = resolve_iterator_source(config, payload)?;
        let rounds = source.as_array().map(Vec::len).unwrap_or_default() as u32;
        // v1 serial execution: the current round is the highest round with any region row
        // (ADR D2); settled rounds keep their rows, so this is monotonic.
        let Some(current) = region_rows.iter().filter_map(|row| row.iteration).max() else {
            // Region rows always carry their round; an absent round means corrupted state.
            return Err(format!(
                "iteration node {} has region rows without a round",
                node.id
            ));
        };

        if ledger.contains_key(&current) {
            // The current round is settled: a failed round under `fail` propagates (this also
            // replays the crash window between settlement and node failure), otherwise the
            // node starts the next round or completes with the ledger projection.
            if config.error_strategy == IterationErrorStrategy::Fail
                && matches!(ledger.get(&current), Some(RoundOutcome::Failed { .. }))
            {
                let error = match ledger.get(&current) {
                    Some(RoundOutcome::Failed { error, .. }) => error.clone(),
                    _ => unreachable!("checked Failed above"),
                };
                return Ok(CompositeAdvancePlan::FailNode { error });
            }
            if current + 1 < rounds {
                let item = source
                    .as_array()
                    .and_then(|items| items.get(current as usize + 1))
                    .cloned()
                    .unwrap_or(Value::Null);
                let round = current + 1;
                let decisions = payload.iteration_round_decisions(round);
                let projection = BranchProjection::new_region_round(
                    graph, &node.id, region, round, node_runs, &decisions,
                );
                let node_ids: Vec<String> = projection
                    .ready_nodes()
                    .iter()
                    .map(|ready| ready.id.clone())
                    .collect();
                return Ok(CompositeAdvancePlan::StartRound {
                    round,
                    item,
                    node_ids,
                });
            }
            // Every round settled: complete with the ledger-derived exposed variables.
            let (output, entries, failed_count) = project_exposed_values(&ledger);
            let exposed = vec![
                (format!("{}.output", node.id), output.clone()),
                (format!("{}.entries", node.id), entries),
                (format!("{}.failed_count", node.id), failed_count),
            ];
            return Ok(CompositeAdvancePlan::CompleteNode {
                exposed,
                output: Some(serde_json::to_string(&output).unwrap_or_default()),
            });
        }

        // The current round has not settled: drive it, or settle it once drained.
        let decisions = payload.iteration_round_decisions(current);
        let projection = BranchProjection::new_region_round(
            graph, &node.id, region, current, node_runs, &decisions,
        );
        if projection.has_in_flight() {
            return Ok(CompositeAdvancePlan::Noop);
        }
        let ready: Vec<String> = projection
            .ready_nodes()
            .iter()
            .map(|ready| ready.id.clone())
            .collect();
        if !ready.is_empty() {
            return Ok(CompositeAdvancePlan::StartRegionNodes { node_ids: ready });
        }

        // Drained: settle the round. The structural check reads this round's rows only, so a
        // previous round's pool value can never impersonate this round's output (ADR D3).
        let item = source
            .as_array()
            .and_then(|items| items.get(current as usize))
            .cloned()
            .unwrap_or(Value::Null);
        let entry = settle_round_outcome(config, &region_rows, current, item, payload)?;
        let continuation = match (&entry, config.error_strategy) {
            (RoundOutcome::Failed { error, .. }, IterationErrorStrategy::Fail) => {
                CompositeContinuation::Fail {
                    error: format!("iteration round {current} failed: {error}"),
                }
            }
            _ if current + 1 < rounds => {
                let next = current + 1;
                let item = source
                    .as_array()
                    .and_then(|items| items.get(next as usize))
                    .cloned()
                    .unwrap_or(Value::Null);
                let decisions = payload.iteration_round_decisions(next);
                let projection = BranchProjection::new_region_round(
                    graph, &node.id, region, next, node_runs, &decisions,
                );
                let node_ids: Vec<String> = projection
                    .ready_nodes()
                    .iter()
                    .map(|ready| ready.id.clone())
                    .collect();
                CompositeContinuation::StartNextRound {
                    round: next,
                    item,
                    node_ids,
                }
            }
            _ => {
                let (output, entries, failed_count) = {
                    let mut complete = ledger;
                    complete.insert(current, entry.clone());
                    project_exposed_values(&complete)
                };
                let exposed = vec![
                    (format!("{}.output", node.id), output.clone()),
                    (format!("{}.entries", node.id), entries),
                    (format!("{}.failed_count", node.id), failed_count),
                ];
                CompositeContinuation::Complete {
                    exposed,
                    output: Some(serde_json::to_string(&output).unwrap_or_default()),
                }
            }
        };
        Ok(CompositeAdvancePlan::SettleRound {
            round: current,
            entry,
            continuation,
        })
    }
}
/// Plans the startup boundary for a fresh iteration node-run (ADR D2).
///
/// The safety ceiling is checked before any round executes — never truncating silently and
/// never spending rounds first — and an empty source completes immediately with empty outputs.
fn plan_startup(
    node: &WorkflowGraphNode,
    config: &IterationConfig,
    graph: &WorkflowGraph,
    region: &crate::workflow_run::engine::iteration::CompositeRegion,
    node_runs: &[WorkflowNodeRun],
    payload: &WorkflowRunPayload,
) -> Result<CompositeAdvancePlan, String> {
    let source = resolve_iterator_source(config, payload)?;
    let Some(items) = source.as_array() else {
        return Err(format!(
            "iteration source {} did not resolve to an array",
            config.iterator_selector.qualified()
        ));
    };
    let rounds = items.len() as u32;
    if rounds > config.max_iterations {
        return Err(format!(
            "iteration source {} has {rounds} elements, exceeding maxIterations {}; raise maxIterations or shorten the source array",
            config.iterator_selector.qualified(),
            config.max_iterations
        ));
    }
    if rounds == 0 {
        // An empty source is a legal input: the node succeeds with empty outputs and no
        // region row ever exists (ADR D1).
        return Ok(CompositeAdvancePlan::CompleteNode {
            exposed: vec![
                (format!("{}.output", node.id), Value::Array(Vec::new())),
                (format!("{}.entries", node.id), Value::Array(Vec::new())),
                (format!("{}.failed_count", node.id), serde_json::json!(0)),
            ],
            output: Some("[]".to_string()),
        });
    }
    let decisions = payload.iteration_round_decisions(0);
    let projection =
        BranchProjection::new_region_round(graph, &node.id, region, 0, node_runs, &decisions);
    let node_ids: Vec<String> = projection
        .ready_nodes()
        .iter()
        .map(|ready| ready.id.clone())
        .collect();
    Ok(CompositeAdvancePlan::StartRound {
        round: 0,
        item: items[0].clone(),
        node_ids,
    })
}

/// Resolves the iterator source value from the committed pool; an unassigned or unresolvable
/// source is an iteration-own failure (ADR D1, D8).
fn resolve_iterator_source(
    config: &IterationConfig,
    payload: &WorkflowRunPayload,
) -> Result<Value, String> {
    let selector = &config.iterator_selector;
    let value = payload
        .variable_pool
        .resolve(selector)
        .map_err(|error| format!("iteration source cannot be resolved: {error}"))?
        .cloned()
        .ok_or_else(|| {
            format!(
                "iteration source {} is not assigned yet",
                selector.qualified()
            )
        })?;
    Ok(value)
}

/// Computes the settled outcome of one drained round from that round's rows and the pool.
///
/// A failed round row records the row's error. Otherwise the structural check requires a
/// `Succeeded` row for the collect target in this round; a round whose branch bypassed the
/// collect target settles as failed rather than reading the previous round's stale pool value
/// (ADR D3).
fn settle_round_outcome(
    config: &IterationConfig,
    region_rows: &[&WorkflowNodeRun],
    round: u32,
    item: Value,
    payload: &WorkflowRunPayload,
) -> Result<RoundOutcome, String> {
    let round_rows: Vec<&&WorkflowNodeRun> = region_rows
        .iter()
        .filter(|row| row.iteration == Some(round))
        .collect();
    if let Some(failed) = round_rows
        .iter()
        .find(|row| row.status == WorkflowNodeStatus::Failed)
    {
        return Ok(RoundOutcome::Failed {
            item,
            error: failed.error.clone().unwrap_or_default(),
        });
    }
    let collect = &config.collect_selector;
    let collect_succeeded = round_rows
        .iter()
        .any(|row| row.node_id == collect.node_id && row.status == WorkflowNodeStatus::Succeeded);
    if !collect_succeeded {
        return Ok(RoundOutcome::Failed {
            item,
            error: "collect target did not run this round".to_string(),
        });
    }
    let output = payload
        .variable_pool
        .resolve(collect)
        .map_err(|error| format!("collect target cannot be resolved: {error}"))?
        .cloned()
        .ok_or_else(|| {
            format!(
                "collect target {} succeeded but produced no value this round",
                collect.qualified()
            )
        })?;
    Ok(RoundOutcome::Succeeded { item, output })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_run::engine::WorkflowRunPayload;
    use crate::workflow_run::engine::branch_projection::ProjectedNodeState;
    use crate::workflow_run::engine::graph::WorkflowGraph;
    use crate::workflow_run::engine::variable_pool::WorkflowVariablePool;
    use ora_contracts::WorkflowRunLocale;
    use ora_domain::{AuditFields, WorkflowNodeRunId, WorkflowRunId};
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// start → iter [fix, check] → out, with `fix` collecting `fix.output` per round.
    fn iteration_graph(error_strategy: &str, max_iterations: u32) -> WorkflowGraph {
        WorkflowGraph::parse(&json!({
            "nodes": [
                { "id": "start", "data": { "kind": "start", "inputVariables": [
                    { "name": "prs", "valueType": "array[object]" }
                ] } },
                { "id": "iter", "data": { "kind": "iteration", "iterationConfig": {
                    "iteratorSelector": ["start", "prs"],
                    "collectSelector": ["fix", "output"],
                    "errorStrategy": error_strategy,
                    "maxIterations": max_iterations
                } } },
                { "id": "fix", "parentId": "iter", "data": { "kind": "agent", "agentConfig": {
                    "executor": { "agentCli": "c", "modelId": "m" }, "prompt": "fix {{#iter.item#}}"
                } } },
                { "id": "check", "parentId": "iter", "data": { "kind": "condition", "cases": [] } },
                { "id": "out", "data": { "kind": "output" } }
            ],
            "edges": [
                { "source": "start", "target": "iter" },
                { "source": "iter", "target": "fix" },
                { "source": "fix", "target": "check" },
                { "source": "iter", "target": "out" }
            ]
        }).to_string()).unwrap()
    }

    fn payload_with_source(items: Value) -> WorkflowRunPayload {
        let graph = iteration_graph("fail", 50);
        let mut pool = WorkflowVariablePool::from_graph(&graph);
        pool.set("start.prs", "start", items).unwrap();
        let mut payload = WorkflowRunPayload::new(WorkflowRunLocale::EnUs, Default::default());
        payload.variable_pool = pool;
        payload
    }

    fn round_row(node_id: &str, round: u32, status: WorkflowNodeStatus) -> WorkflowNodeRun {
        WorkflowNodeRun::new(
            WorkflowNodeRunId::new(format!("{node_id}-{round}")),
            WorkflowRunId::new("run-1"),
            node_id,
            "agent",
            None,
            status,
            None,
            None,
            None,
            None,
            Some(1),
            None,
            AuditFields::new(1, 1, false),
        )
        .in_iteration(Some(round))
    }

    fn plan(
        graph: &WorkflowGraph,
        node_runs: &[WorkflowNodeRun],
        payload: &WorkflowRunPayload,
    ) -> Result<CompositeAdvancePlan, String> {
        let node = graph.node("iter").unwrap();
        IterationRuntime.plan_advance(node, graph, node_runs, payload)
    }

    /// A fresh node over a two-element source starts round 0 at the entry target only.
    #[test]
    fn startup_plans_round_zero_at_the_entry_targets() {
        let graph = iteration_graph("fail", 50);
        let payload = payload_with_source(json!([{ "id": 1 }, { "id": 2 }]));
        assert_eq!(
            plan(&graph, &[], &payload).unwrap(),
            CompositeAdvancePlan::StartRound {
                round: 0,
                item: json!({ "id": 1 }),
                node_ids: vec!["fix".to_string()],
            }
        );
    }

    /// An empty source completes immediately with empty exposed variables and no round.
    #[test]
    fn startup_completes_immediately_on_an_empty_source() {
        let graph = iteration_graph("fail", 50);
        let payload = payload_with_source(json!([]));
        assert_eq!(
            plan(&graph, &[], &payload).unwrap(),
            CompositeAdvancePlan::CompleteNode {
                exposed: vec![
                    ("iter.output".to_string(), json!([])),
                    ("iter.entries".to_string(), json!([])),
                    ("iter.failed_count".to_string(), json!(0)),
                ],
                output: Some("[]".to_string()),
            }
        );
    }

    /// A source longer than the ceiling fails at the startup boundary with both numbers in the
    /// message, before any round executes.
    #[test]
    fn startup_fails_when_the_source_exceeds_the_ceiling() {
        let graph = iteration_graph("fail", 1);
        let payload = payload_with_source(json!([{ "id": 1 }, { "id": 2 }]));
        let error = plan(&graph, &[], &payload).unwrap_err();
        assert!(error.contains("2 elements"), "{error}");
        assert!(error.contains("maxIterations 1"), "{error}");
    }

    /// A non-array source is an iteration-own failure even before any round runs.
    #[test]
    fn startup_fails_when_the_source_is_not_an_array() {
        // The graph parser rejects a non-array iterator selector, so the runtime-level failure
        // can only be exercised with a hand-retargeted config over a string pool value — the
        // same code path a corrupted pool would take.
        let error = plan_startup_with_string_source();
        assert!(error.contains("did not resolve to an array"), "{error}");
    }

    /// Builds the non-array planning error directly (see the test above for why).
    fn plan_startup_with_string_source() -> String {
        let graph = iteration_graph("fail", 50);
        let node = graph.node("iter").unwrap().clone();
        let region = graph.region("iter").unwrap().clone();
        let config = node.iteration_config.as_ref().unwrap();
        let mut pool = WorkflowVariablePool::from_graph(&graph);
        pool.declare("start.text", "string", "start");
        pool.set("start.text", "start", json!("nope")).unwrap();
        let mut payload = WorkflowRunPayload::new(WorkflowRunLocale::EnUs, Default::default());
        payload.variable_pool = pool;
        let mut string_config = config.clone();
        string_config.iterator_selector =
            crate::workflow_run::engine::variable_pool::VariableSelector::new(
                "start".to_string(),
                "text".to_string(),
                Vec::new(),
            );
        plan_startup(&node, &string_config, &graph, &region, &[], &payload).unwrap_err()
    }

    /// An in-flight round is a no-op; newly ready members start; a drained round settles with
    /// the collect target's pool value.
    #[test]
    fn an_in_flight_round_is_a_noop_and_ready_members_start() {
        let graph = iteration_graph("fail", 50);
        let payload = payload_with_source(json!([{ "id": 1 }]));
        // fix finished, check still running: the round is in flight.
        let node_runs = vec![
            round_row("fix", 0, WorkflowNodeStatus::Succeeded),
            round_row("check", 0, WorkflowNodeStatus::Running),
        ];
        assert_eq!(
            plan(&graph, &node_runs, &payload).unwrap(),
            CompositeAdvancePlan::Noop
        );

        // fix finished, check finished as a swift row already resolved by a prior wave: the
        // drained round settles with the collect target's committed pool value.
        let mut payload = payload;
        payload
            .variable_pool
            .set("fix.output", "fix", json!("fix output"))
            .unwrap();
        let node_runs = vec![
            round_row("fix", 0, WorkflowNodeStatus::Succeeded),
            round_row("check", 0, WorkflowNodeStatus::Succeeded),
        ];
        match plan(&graph, &node_runs, &payload).unwrap() {
            CompositeAdvancePlan::SettleRound { round, entry, .. } => {
                assert_eq!(round, 0);
                assert_eq!(
                    entry,
                    RoundOutcome::Succeeded {
                        item: json!({ "id": 1 }),
                        output: json!("fix output"),
                    }
                );
            }
            other => panic!("expected settlement, got {other:?}"),
        }
    }

    /// The settlement reads this round's rows: a branch that bypassed the collect target
    /// settles as a failed round even though the pool holds the previous round's value.
    #[test]
    fn settlement_requires_the_collect_target_to_have_run_this_round() {
        // The region's condition gates the collect target: a round that picks the else branch
        // bypasses `fix`, whose stale round-0 pool value must not impersonate round 1's output.
        let graph = WorkflowGraph::parse(
            &json!({
                "nodes": [
                    { "id": "start", "data": { "kind": "start", "inputVariables": [
                        { "name": "prs", "valueType": "array[object]" }
                    ] } },
                    { "id": "iter", "data": { "kind": "iteration", "iterationConfig": {
                        "iteratorSelector": ["start", "prs"],
                        "collectSelector": ["fix", "output"],
                        "errorStrategy": "continue"
                    } } },
                    { "id": "gate", "parentId": "iter", "data": { "kind": "condition", "cases": [
                        { "id": "fix-it", "logic": "and", "conditions": [] }
                    ] } },
                    { "id": "fix", "parentId": "iter", "data": { "kind": "agent", "agentConfig": {
                        "executor": { "agentCli": "c", "modelId": "m" }, "prompt": "fix"
                    } } }
                ],
                "edges": [
                    { "source": "start", "target": "iter" },
                    { "source": "iter", "target": "gate" },
                    { "source": "gate", "sourceHandle": "fix-it", "target": "fix" }
                ]
            })
            .to_string(),
        )
        .unwrap();
        let mut payload = payload_with_source(json!([{ "id": 1 }, { "id": 2 }]));
        // The pool carries fix.output from round 0, but round 1's branch never ran fix.
        payload
            .variable_pool
            .set("fix.output", "fix", json!("stale round 0 value"))
            .unwrap();
        payload
            .record_round_outcome(
                "iter",
                0,
                RoundOutcome::Succeeded {
                    item: json!({ "id": 1 }),
                    output: json!("round 0 value"),
                },
            )
            .unwrap();
        // Round 0 selected the fixing branch; round 1 selected else and bypassed fix.
        payload
            .iteration_condition_decisions
            .insert("gate#0".to_string(), "fix-it".to_string());
        payload
            .iteration_condition_decisions
            .insert("gate#1".to_string(), "else".to_string());
        let node_runs = vec![
            round_row("gate", 0, WorkflowNodeStatus::Succeeded),
            round_row("fix", 0, WorkflowNodeStatus::Succeeded),
            round_row("gate", 1, WorkflowNodeStatus::Succeeded),
        ];
        match plan(&graph, &node_runs, &payload).unwrap() {
            CompositeAdvancePlan::SettleRound { round, entry, .. } => {
                assert_eq!(round, 1);
                assert_eq!(
                    entry,
                    RoundOutcome::Failed {
                        item: json!({ "id": 2 }),
                        error: "collect target did not run this round".to_string(),
                    }
                );
            }
            other => panic!("expected settlement, got {other:?}"),
        }
    }

    /// A failed round under `continue` settles into the ledger and starts the next round; the
    /// same failure under `fail` propagates as a node failure.
    #[test]
    fn failed_rounds_follow_the_error_strategy() {
        let node_runs = vec![round_row("fix", 0, WorkflowNodeStatus::Failed)];

        let graph = iteration_graph("continue", 50);
        let payload = payload_with_source(json!([{ "id": 1 }, { "id": 2 }]));
        match plan(&graph, &node_runs, &payload).unwrap() {
            CompositeAdvancePlan::SettleRound {
                round,
                entry,
                continuation,
            } => {
                assert_eq!(round, 0);
                assert!(matches!(entry, RoundOutcome::Failed { .. }));
                assert!(matches!(
                    continuation,
                    CompositeContinuation::StartNextRound { round: 1, .. }
                ));
            }
            other => panic!("expected settlement, got {other:?}"),
        }

        let graph = iteration_graph("fail", 50);
        let payload = payload_with_source(json!([{ "id": 1 }, { "id": 2 }]));
        match plan(&graph, &node_runs, &payload).unwrap() {
            CompositeAdvancePlan::SettleRound { continuation, .. } => {
                assert!(matches!(continuation, CompositeContinuation::Fail { .. }));
            }
            other => panic!("expected settlement, got {other:?}"),
        }
    }

    /// The last settled round completes the node with the ledger projection; `fail` replays a
    /// settled failed round as a node failure even after a crash between the two transactions.
    #[test]
    fn a_settled_final_round_completes_and_a_settled_failure_replays() {
        let graph = iteration_graph("continue", 50);
        let mut payload = payload_with_source(json!([{ "id": 1 }, { "id": 2 }]));
        payload
            .record_round_outcome(
                "iter",
                0,
                RoundOutcome::Succeeded {
                    item: json!({ "id": 1 }),
                    output: json!("one"),
                },
            )
            .unwrap();
        payload
            .record_round_outcome(
                "iter",
                1,
                RoundOutcome::Succeeded {
                    item: json!({ "id": 2 }),
                    output: json!("two"),
                },
            )
            .unwrap();
        let node_runs = vec![
            round_row("fix", 0, WorkflowNodeStatus::Succeeded),
            round_row("fix", 1, WorkflowNodeStatus::Succeeded),
        ];
        match plan(&graph, &node_runs, &payload).unwrap() {
            CompositeAdvancePlan::CompleteNode { exposed, output } => {
                assert_eq!(
                    exposed,
                    vec![
                        ("iter.output".to_string(), json!(["one", "two"])),
                        (
                            "iter.entries".to_string(),
                            json!([
                                { "item": { "id": 1 }, "status": "succeeded", "output": "one", "error": null },
                                { "item": { "id": 2 }, "status": "succeeded", "output": "two", "error": null },
                            ])
                        ),
                        ("iter.failed_count".to_string(), json!(0)),
                    ]
                );
                assert_eq!(output, Some(r#"["one","two"]"#.to_string()));
            }
            other => panic!("expected completion, got {other:?}"),
        }

        // `fail` + a settled failed round: replanning after a crash between settlement and the
        // node's failure still fails the node instead of starting the next round.
        let graph = iteration_graph("fail", 50);
        let mut payload = payload_with_source(json!([{ "id": 1 }, { "id": 2 }]));
        payload
            .record_round_outcome(
                "iter",
                0,
                RoundOutcome::Failed {
                    item: json!({ "id": 1 }),
                    error: "agent exploded".to_string(),
                },
            )
            .unwrap();
        let node_runs = vec![round_row("fix", 0, WorkflowNodeStatus::Failed)];
        match plan(&graph, &node_runs, &payload).unwrap() {
            CompositeAdvancePlan::FailNode { error } => {
                assert!(error.contains("agent exploded"), "{error}");
            }
            other => panic!("expected node failure, got {other:?}"),
        }
    }

    /// The scoped projection keeps rounds isolated: round-1 rows do not leak into round 0.
    #[test]
    fn the_region_round_projection_is_isolated_per_round() {
        let graph = iteration_graph("fail", 50);
        let payload = payload_with_source(json!([{ "id": 1 }, { "id": 2 }]));
        let decisions = payload.iteration_round_decisions(0);
        let region = graph.region("iter").unwrap();
        let node_runs = vec![
            round_row("fix", 0, WorkflowNodeStatus::Running),
            round_row("fix", 1, WorkflowNodeStatus::Succeeded),
        ];
        let projection =
            BranchProjection::new_region_round(&graph, "iter", region, 0, &node_runs, &decisions);
        assert_eq!(projection.state("fix"), ProjectedNodeState::Running);
        assert_eq!(projection.state("check"), ProjectedNodeState::NotReached);
        assert!(projection.has_in_flight());
    }
}
