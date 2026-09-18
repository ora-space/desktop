//! Pure validation that a newer published snapshot can take over a failed or cancelled run.

use super::graph::WorkflowGraph;
use super::variable_pool::WorkflowVariablePool;
use ora_domain::{WorkflowNodeRun, WorkflowNodeStatus};
use std::collections::{BTreeSet, HashSet};

/// Migrated variable pool that a compatible snapshot switch may persist onto the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotSwitchPlan {
    pub variable_pool: WorkflowVariablePool,
}

/// Machine-readable reason a published snapshot cannot resume this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotIncompatibility {
    pub reason: String,
}

/// Checks that `new_graph` can take over a run whose live node runs are `node_runs`, and builds the
/// migrated variable pool. `node_runs` are the live (not soft-deleted) rows.
pub fn plan_snapshot_switch(
    old_graph: &WorkflowGraph,
    new_graph: &WorkflowGraph,
    node_runs: &[WorkflowNodeRun],
    pool: &WorkflowVariablePool,
) -> Result<SnapshotSwitchPlan, SnapshotIncompatibility> {
    for node_run in node_runs {
        let Some(new_node) = new_graph.node(&node_run.node_id) else {
            return Err(incompatible(format!("node_missing:{}", node_run.node_id)));
        };
        if new_node.node_type.as_str() != node_run.node_type {
            return Err(incompatible(format!(
                "node_type_changed:{}",
                node_run.node_id
            )));
        }
    }

    let old_start = old_graph.start_node();
    let new_start = new_graph.start_node();
    match (old_start, new_start) {
        (Some(old_start), Some(new_start)) if old_start.id == new_start.id => {
            let old_variables = start_variable_set(old_start);
            let new_variables = start_variable_set(new_start);
            if old_variables != new_variables {
                return Err(incompatible("start_variables_changed"));
            }
        }
        _ => return Err(incompatible("start_node_changed")),
    }

    let succeeded_writers: HashSet<&str> = node_runs
        .iter()
        .filter(|node_run| node_run.status == WorkflowNodeStatus::Succeeded)
        .map(|node_run| node_run.node_id.as_str())
        .collect();
    let start_id = old_start.map(|node| node.id.as_str());
    let mut new_pool = WorkflowVariablePool::from_graph(new_graph);
    for (selector, value) in &pool.values {
        if !should_carry(selector, pool, &succeeded_writers, start_id) {
            continue;
        }
        let Some(new_definition) = new_pool.catalog.get(selector) else {
            return Err(incompatible(format!("variable_missing:{selector}")));
        };
        if let Some(old_definition) = pool.catalog.get(selector)
            && old_definition.value_type != new_definition.value_type
        {
            return Err(incompatible(format!("variable_type_changed:{selector}")));
        }
        new_pool.values.insert(selector.clone(), value.clone());
    }
    new_pool.revision = pool.revision.saturating_add(1);

    let resume_unit_owners: HashSet<String> = node_runs
        .iter()
        .filter(|node_run| {
            matches!(
                node_run.status,
                WorkflowNodeStatus::Failed | WorkflowNodeStatus::Cancelled
            )
        })
        .filter_map(|node_run| super::region::resume_unit_owner_id(old_graph, &node_run.node_id))
        .collect();
    for node in old_graph.nodes() {
        let Some(old_region) = old_graph.region(&node.id) else {
            continue;
        };
        if resume_unit_owners.contains(&node.id) {
            continue;
        }
        let succeeded = node_runs.iter().any(|row| {
            row.node_id == node.id
                && row.iteration.is_none()
                && row.status == WorkflowNodeStatus::Succeeded
        });
        if !succeeded {
            continue;
        }
        let old_members: HashSet<&str> = old_region.member_ids.iter().map(String::as_str).collect();
        let new_members: HashSet<&str> = new_graph
            .region(&node.id)
            .map(|region| region.member_ids.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let old_config = node.iteration_config.as_ref();
        let new_config = new_graph
            .node(&node.id)
            .and_then(|node| node.iteration_config.as_ref());
        if old_config != new_config || old_members != new_members {
            return Err(incompatible(format!(
                "iteration node {} changed after it completed",
                node.id
            )));
        }
    }

    Ok(SnapshotSwitchPlan {
        variable_pool: new_pool,
    })
}

fn incompatible(reason: impl Into<String>) -> SnapshotIncompatibility {
    SnapshotIncompatibility {
        reason: reason.into(),
    }
}

fn start_variable_set(start: &super::graph::WorkflowGraphNode) -> BTreeSet<(String, String, bool)> {
    start
        .input_variables
        .iter()
        .map(|variable| {
            (
                variable.name.clone(),
                variable.value_type.clone(),
                variable.required,
            )
        })
        .collect()
}

fn should_carry(
    selector: &str,
    pool: &WorkflowVariablePool,
    succeeded_writers: &HashSet<&str>,
    start_id: Option<&str>,
) -> bool {
    if selector.starts_with("sys.") {
        return true;
    }
    let Some(definition) = pool.catalog.get(selector) else {
        return false;
    };
    if definition.writer == "sys" || definition.writer == "global" {
        return true;
    }
    if start_id == Some(definition.writer.as_str()) {
        return true;
    }
    succeeded_writers.contains(definition.writer.as_str())
}

#[cfg(test)]
mod tests {
    use super::{SnapshotIncompatibility, plan_snapshot_switch};
    use crate::workflow_run::engine::variable_pool::WorkflowVariablePool;
    use ora_domain::{
        AuditFields, WorkflowNodeRun, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    const IDENTICAL: &str = r#"{
        "nodes": [
            {"id":"start","data":{"kind":"start"}},
            {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
            {"id":"out","data":{"kind":"output"}}
        ],
        "edges": [
            {"source":"start","target":"a"},
            {"source":"a","target":"out"}
        ]
    }"#;

    const WITH_B: &str = r#"{
        "nodes": [
            {"id":"start","data":{"kind":"start"}},
            {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
            {"id":"b","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"b"}}},
            {"id":"out","data":{"kind":"output"}}
        ],
        "edges": [
            {"source":"start","target":"a"},
            {"source":"a","target":"b"},
            {"source":"b","target":"out"}
        ]
    }"#;

    const WITHOUT_B: &str = r#"{
        "nodes": [
            {"id":"start","data":{"kind":"start"}},
            {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
            {"id":"out","data":{"kind":"output"}}
        ],
        "edges": [
            {"source":"start","target":"a"},
            {"source":"a","target":"out"}
        ]
    }"#;

    const A_AS_CONDITION: &str = r#"{
        "nodes": [
            {"id":"start","data":{"kind":"start"}},
            {"id":"a","data":{"kind":"condition"}},
            {"id":"out","data":{"kind":"output"}}
        ],
        "edges": [
            {"source":"start","target":"a"},
            {"source":"a","target":"out"}
        ]
    }"#;

    const START_WITH_TOPIC: &str = r#"{
        "nodes": [
            {"id":"start","data":{"kind":"start","inputVariables":[{"name":"topic","valueType":"string","required":true}]}},
            {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
            {"id":"out","data":{"kind":"output"}}
        ],
        "edges": [
            {"source":"start","target":"a"},
            {"source":"a","target":"out"}
        ]
    }"#;

    const C_STRUCTURED: &str = r#"{
        "nodes": [
            {"id":"start","data":{"kind":"start"}},
            {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"a"}}},
            {"id":"c","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"c",
                "outputContract":{"type":"structured","textExposure":"includeFinalText",
                    "schema":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}}}}},
            {"id":"out","data":{"kind":"output"}}
        ],
        "edges": [
            {"source":"start","target":"a"},
            {"source":"a","target":"c"},
            {"source":"c","target":"out"}
        ]
    }"#;

    const C_PLAIN: &str = r#"{
        "nodes": [
            {"id":"start","data":{"kind":"start"}},
            {"id":"a","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"fixed"}}},
            {"id":"c","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"fixed"}}},
            {"id":"out","data":{"kind":"output"}}
        ],
        "edges": [
            {"source":"start","target":"a"},
            {"source":"a","target":"c"},
            {"source":"c","target":"out"}
        ]
    }"#;

    fn parse(json: &str) -> crate::workflow_run::engine::graph::WorkflowGraph {
        crate::workflow_run::engine::graph::WorkflowGraph::parse(json).expect("graph")
    }

    fn node_run(node_id: &str, node_type: &str, status: WorkflowNodeStatus) -> WorkflowNodeRun {
        WorkflowNodeRun::new(
            WorkflowNodeRunId::new(format!("nr-{node_id}")),
            WorkflowRunId::new("run-1"),
            ora_domain::WorkflowScopeId::new("root:test"),
            node_id,
            node_type,
            None,
            status,
            None,
            None,
            None,
            None,
            Some(1),
            Some(2),
            AuditFields::new(1, 1, false),
        )
    }

    /// (a) Identical graphs keep carried values and only bump the pool revision.
    #[test]
    fn identical_graphs_carry_the_pool_and_increment_revision() {
        let graph = parse(IDENTICAL);
        let mut pool = WorkflowVariablePool::from_graph(&graph);
        pool.values.insert("a.output".to_string(), json!("kept"));
        pool.revision = 4;
        let node_runs = vec![
            node_run("start", "start", WorkflowNodeStatus::Succeeded),
            node_run("a", "agent", WorkflowNodeStatus::Succeeded),
        ];
        let plan = plan_snapshot_switch(&graph, &graph, &node_runs, &pool).unwrap();
        let mut expected = WorkflowVariablePool::from_graph(&graph);
        expected
            .values
            .insert("a.output".to_string(), json!("kept"));
        expected.revision = 5;
        assert_eq!(plan.variable_pool, expected);
    }

    /// (b) Dropping a node that already succeeded is refused.
    #[test]
    fn dropping_a_succeeded_node_is_node_missing() {
        let old_graph = parse(WITH_B);
        let new_graph = parse(WITHOUT_B);
        let pool = WorkflowVariablePool::from_graph(&old_graph);
        let node_runs = vec![
            node_run("start", "start", WorkflowNodeStatus::Succeeded),
            node_run("a", "agent", WorkflowNodeStatus::Succeeded),
            node_run("b", "agent", WorkflowNodeStatus::Succeeded),
        ];
        assert_eq!(
            plan_snapshot_switch(&old_graph, &new_graph, &node_runs, &pool).unwrap_err(),
            SnapshotIncompatibility {
                reason: "node_missing:b".to_string(),
            }
        );
    }

    /// (c) Keeping a node id while changing its type is refused.
    #[test]
    fn changing_a_live_node_type_is_node_type_changed() {
        let old_graph = parse(IDENTICAL);
        let new_graph = parse(A_AS_CONDITION);
        let pool = WorkflowVariablePool::from_graph(&old_graph);
        let node_runs = vec![
            node_run("start", "start", WorkflowNodeStatus::Succeeded),
            node_run("a", "agent", WorkflowNodeStatus::Succeeded),
        ];
        assert_eq!(
            plan_snapshot_switch(&old_graph, &new_graph, &node_runs, &pool).unwrap_err(),
            SnapshotIncompatibility {
                reason: "node_type_changed:a".to_string(),
            }
        );
    }

    /// (d) Adding a Start variable changes the kickoff contract.
    #[test]
    fn adding_a_start_variable_is_start_variables_changed() {
        let old_graph = parse(IDENTICAL);
        let new_graph = parse(START_WITH_TOPIC);
        let pool = WorkflowVariablePool::from_graph(&old_graph);
        let node_runs = vec![node_run("start", "start", WorkflowNodeStatus::Succeeded)];
        assert_eq!(
            plan_snapshot_switch(&old_graph, &new_graph, &node_runs, &pool).unwrap_err(),
            SnapshotIncompatibility {
                reason: "start_variables_changed".to_string(),
            }
        );
    }

    /// (e) A failed node's structured-output contract may appear or disappear because its values
    /// are dropped on resume anyway.
    #[test]
    fn failed_structured_output_contract_changes_are_compatible() {
        let old_graph = parse(C_STRUCTURED);
        let new_graph = parse(C_PLAIN);
        let mut pool = WorkflowVariablePool::from_graph(&old_graph);
        pool.values
            .insert("c.structured_output".to_string(), json!({"name": "stale"}));
        let node_runs = vec![
            node_run("start", "start", WorkflowNodeStatus::Succeeded),
            node_run("a", "agent", WorkflowNodeStatus::Succeeded),
            node_run("c", "agent", WorkflowNodeStatus::Failed),
        ];
        let plan = plan_snapshot_switch(&old_graph, &new_graph, &node_runs, &pool).unwrap();
        assert!(
            !plan
                .variable_pool
                .values
                .contains_key("c.structured_output")
        );
        assert!(
            !plan
                .variable_pool
                .catalog
                .contains_key("c.structured_output")
        );
    }

    /// (f) A succeeded writer's declared type must match in the new catalog.
    #[test]
    fn succeeded_variable_type_change_is_refused() {
        let old_graph = parse(IDENTICAL);
        let new_graph = parse(IDENTICAL);
        let mut pool = WorkflowVariablePool::from_graph(&old_graph);
        if let Some(definition) = pool.catalog.get_mut("a.output") {
            definition.value_type = "number".to_string();
        }
        pool.values.insert("a.output".to_string(), json!(1));
        let node_runs = vec![
            node_run("start", "start", WorkflowNodeStatus::Succeeded),
            node_run("a", "agent", WorkflowNodeStatus::Succeeded),
        ];
        assert_eq!(
            plan_snapshot_switch(&old_graph, &new_graph, &node_runs, &pool).unwrap_err(),
            SnapshotIncompatibility {
                reason: "variable_type_changed:a.output".to_string(),
            }
        );
    }

    /// (g) Failed and cancelled writers are dropped; `sys.*` and Start values are kept.
    #[test]
    fn failed_writers_are_dropped_and_system_and_start_values_are_kept() {
        let graph = parse(WITH_B);
        let mut pool = WorkflowVariablePool::from_graph(&graph);
        pool.values
            .insert("sys.workflow_id".to_string(), json!("workflow-1"));
        pool.values
            .insert("start.input".to_string(), json!("kickoff"));
        pool.values.insert("a.output".to_string(), json!("from-a"));
        pool.values
            .insert("b.output".to_string(), json!("from-cancelled-b"));
        let node_runs = vec![
            node_run("start", "start", WorkflowNodeStatus::Succeeded),
            node_run("a", "agent", WorkflowNodeStatus::Succeeded),
            node_run("b", "agent", WorkflowNodeStatus::Cancelled),
        ];
        let plan = plan_snapshot_switch(&graph, &graph, &node_runs, &pool).unwrap();
        assert_eq!(
            plan.variable_pool.values.get("sys.workflow_id"),
            Some(&json!("workflow-1"))
        );
        assert_eq!(
            plan.variable_pool.values.get("start.input"),
            Some(&json!("kickoff"))
        );
        assert_eq!(
            plan.variable_pool.values.get("a.output"),
            Some(&json!("from-a"))
        );
        assert!(!plan.variable_pool.values.contains_key("b.output"));
    }

    const ITER_MAX_10: &str = r#"{
        "nodes": [
            {"id":"start","data":{"kind":"start","inputVariables":[{"name":"prs","valueType":"array[object]"}]}},
            {"id":"iter","data":{"kind":"iteration","iterationConfig":{
                "iteratorSelector":["start","prs"],
                "collectSelector":["fix","output"],
                "errorStrategy":"fail",
                "maxIterations":10
            }}},
            {"id":"fix","parentId":"iter","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"fix"}}},
            {"id":"out","data":{"kind":"output"}}
        ],
        "edges": [
            {"source":"start","target":"iter"},
            {"source":"iter","target":"fix"},
            {"source":"iter","target":"out"}
        ]
    }"#;

    const ITER_MAX_20: &str = r#"{
        "nodes": [
            {"id":"start","data":{"kind":"start","inputVariables":[{"name":"prs","valueType":"array[object]"}]}},
            {"id":"iter","data":{"kind":"iteration","iterationConfig":{
                "iteratorSelector":["start","prs"],
                "collectSelector":["fix","output"],
                "errorStrategy":"fail",
                "maxIterations":20
            }}},
            {"id":"fix","parentId":"iter","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"fix"}}},
            {"id":"out","data":{"kind":"output"}}
        ],
        "edges": [
            {"source":"start","target":"iter"},
            {"source":"iter","target":"fix"},
            {"source":"iter","target":"out"}
        ]
    }"#;

    const ITER_EXTRA_MEMBER: &str = r#"{
        "nodes": [
            {"id":"start","data":{"kind":"start","inputVariables":[{"name":"prs","valueType":"array[object]"}]}},
            {"id":"iter","data":{"kind":"iteration","iterationConfig":{
                "iteratorSelector":["start","prs"],
                "collectSelector":["fix","output"],
                "errorStrategy":"fail",
                "maxIterations":10
            }}},
            {"id":"fix","parentId":"iter","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"fix"}}},
            {"id":"review","parentId":"iter","data":{"kind":"agent","agentConfig":{"executor":{"agentCli":"c","modelId":"m"},"prompt":"review"}}},
            {"id":"out","data":{"kind":"output"}}
        ],
        "edges": [
            {"source":"start","target":"iter"},
            {"source":"iter","target":"fix"},
            {"source":"fix","target":"review"},
            {"source":"iter","target":"out"}
        ]
    }"#;

    /// A succeeded iteration node that is not the resume unit cannot change after it completed.
    #[test]
    fn succeeded_iteration_config_change_is_incompatible() {
        let old_graph = parse(ITER_MAX_10);
        let new_graph = parse(ITER_MAX_20);
        let pool = WorkflowVariablePool::from_graph(&old_graph);
        let node_runs = vec![
            node_run("start", "start", WorkflowNodeStatus::Succeeded),
            node_run("iter", "iteration", WorkflowNodeStatus::Succeeded),
            node_run("fix", "agent", WorkflowNodeStatus::Succeeded).in_iteration(Some(0)),
            node_run("out", "output", WorkflowNodeStatus::Failed),
        ];
        assert_eq!(
            plan_snapshot_switch(&old_graph, &new_graph, &node_runs, &pool).unwrap_err(),
            SnapshotIncompatibility {
                reason: "iteration node iter changed after it completed".to_string(),
            }
        );
    }

    /// Changing the member set of a succeeded composite is the same incompatibility.
    #[test]
    fn succeeded_iteration_member_set_change_is_incompatible() {
        let old_graph = parse(ITER_MAX_10);
        let new_graph = parse(ITER_EXTRA_MEMBER);
        let pool = WorkflowVariablePool::from_graph(&old_graph);
        let node_runs = vec![
            node_run("start", "start", WorkflowNodeStatus::Succeeded),
            node_run("iter", "iteration", WorkflowNodeStatus::Succeeded),
            node_run("fix", "agent", WorkflowNodeStatus::Succeeded).in_iteration(Some(0)),
            node_run("out", "output", WorkflowNodeStatus::Failed),
        ];
        assert_eq!(
            plan_snapshot_switch(&old_graph, &new_graph, &node_runs, &pool).unwrap_err(),
            SnapshotIncompatibility {
                reason: "iteration node iter changed after it completed".to_string(),
            }
        );
    }

    /// A composite that is itself the resume unit may change freely because the loop restarts.
    #[test]
    fn resume_unit_iteration_config_change_is_compatible() {
        let old_graph = parse(ITER_MAX_10);
        let new_graph = parse(ITER_MAX_20);
        let pool = WorkflowVariablePool::from_graph(&old_graph);
        let node_runs = vec![
            node_run("start", "start", WorkflowNodeStatus::Succeeded),
            node_run("iter", "iteration", WorkflowNodeStatus::Failed),
            node_run("fix", "agent", WorkflowNodeStatus::Succeeded).in_iteration(Some(0)),
            node_run("fix", "agent", WorkflowNodeStatus::Failed).in_iteration(Some(1)),
        ];
        let plan = plan_snapshot_switch(&old_graph, &new_graph, &node_runs, &pool);
        assert!(plan.is_ok(), "{plan:?}");
    }
}
