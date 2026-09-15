//! The iteration composite runtime's graph-level domain: node config, region derivation, and
//! the per-round ledger model.
//!
//! An iteration node (foreach semantics) owns a region: the set of nodes whose React Flow
//! `parentId` points at it. The region's frozen subgraph executes once per element of the
//! iterator source; each round binds `{iter}.item` / `{iter}.index` and settles into a
//! [`RoundOutcome`] ledger entry (ADR "iteration composite runtime" D1–D5).

use crate::workflow_run::engine::graph::{GraphError, WorkflowGraphEdge};
use crate::workflow_run::engine::node_type::NodeType;
use crate::workflow_run::engine::variable_pool::VariableSelector;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

/// The default safety ceiling applied when a node declares no `maxIterations`.
pub const DEFAULT_MAX_ITERATIONS: u32 = 50;

/// How an iteration node reacts when one round fails (ADR "iteration composite runtime" D4).
///
/// The switch only answers "does the node stop after a failed round"; it never changes the
/// shape of the exposed variables.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IterationErrorStrategy {
    /// The first failed round fails the iteration node and the run (the default).
    #[default]
    Fail,
    /// Failed rounds are recorded in the ledger and the remaining rounds still execute.
    Continue,
}

/// The executable configuration of one `iteration` node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IterationConfig {
    /// Selector of the array variable driving the rounds; its length is the round count.
    pub iterator_selector: VariableSelector,
    /// Root variable (inside the region) whose per-round value is collected into
    /// `{iter}.output`.
    pub collect_selector: VariableSelector,
    /// Whether a failed round stops the node or is absorbed into the ledger.
    pub error_strategy: IterationErrorStrategy,
    /// Safety ceiling: a longer iterator source fails the node at the startup boundary.
    /// Must be at least 1.
    pub max_iterations: u32,
}

/// Wire shape of `data.iterationConfig`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WireIterationConfig {
    #[serde(default)]
    iterator_selector: Option<Vec<String>>,
    #[serde(default)]
    collect_selector: Option<Vec<String>>,
    #[serde(default)]
    error_strategy: Option<WireErrorStrategy>,
    #[serde(default)]
    max_iterations: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WireErrorStrategy {
    Fail,
    Continue,
}

impl WireIterationConfig {
    /// Maps the wire config to the domain model, rejecting shapes the runtime cannot honor.
    pub(super) fn into_model(self, node_id: &str) -> Result<IterationConfig, GraphError> {
        let invalid = |reason: String| GraphError::InvalidIteration {
            node_id: node_id.to_string(),
            reason,
        };
        let parse_selector =
            |parts: &Option<Vec<String>>, field: &str| -> Result<VariableSelector, GraphError> {
                let parts = parts
                    .as_ref()
                    .ok_or_else(|| invalid(format!("{field} is required")))?;
                VariableSelector::try_from_parts(parts)
                    .ok_or_else(|| invalid(format!("{field} must be a variable selector")))
            };
        let iterator_selector = parse_selector(&self.iterator_selector, "iteratorSelector")?;
        let collect_selector = parse_selector(&self.collect_selector, "collectSelector")?;
        if !collect_selector.nested.is_empty() {
            return Err(invalid(
                "collectSelector must reference a root variable without a nested path".into(),
            ));
        }
        let error_strategy = match self.error_strategy {
            Some(WireErrorStrategy::Fail) | None => IterationErrorStrategy::Fail,
            Some(WireErrorStrategy::Continue) => IterationErrorStrategy::Continue,
        };
        let max_iterations = self.max_iterations.unwrap_or(DEFAULT_MAX_ITERATIONS);
        if max_iterations < 1 {
            return Err(invalid(format!(
                "maxIterations must be at least 1, got {max_iterations}"
            )));
        }
        Ok(IterationConfig {
            iterator_selector,
            collect_selector,
            error_strategy,
            max_iterations,
        })
    }
}

/// Parses `data.iterationConfig` for one iteration node, rejecting absent or invalid configs.
pub(super) fn parse_iteration_config(
    wire: Option<WireIterationConfig>,
    node_id: &str,
) -> Result<Option<IterationConfig>, GraphError> {
    match wire {
        None => Err(GraphError::InvalidIteration {
            node_id: node_id.to_string(),
            reason: "iteration node requires an iterationConfig".into(),
        }),
        Some(wire) => wire.into_model(node_id).map(Some),
    }
}

/// The region one composite node owns: the member node ids derived from `parentId` containment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositeRegion {
    /// The owning composite node id.
    pub owner_id: String,
    /// Member node ids in graph insertion order.
    pub member_ids: Vec<String>,
}

impl CompositeRegion {
    /// Whether `node_id` is a member of this region.
    pub fn contains(&self, node_id: &str) -> bool {
        self.member_ids.iter().any(|member| member == node_id)
    }
}

/// One settled round of an iteration, keeping illegal states unrepresentable: a succeeded round
/// cannot carry an error and a failed round cannot carry an output (ADR D4). The envelope with
/// all four fields only exists at the variable-pool JSON boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RoundOutcome {
    Succeeded { item: Value, output: Value },
    Failed { item: Value, error: String },
}

impl RoundOutcome {
    /// The round's input element, carried by both variants for ledger projection.
    pub fn item(&self) -> &Value {
        match self {
            Self::Succeeded { item, .. } | Self::Failed { item, .. } => item,
        }
    }

    /// Flattens the round into the `{item, status, output, error}` ledger envelope exposed as
    /// `{iter}.entries`. Nulls keep the four fields addressable for both variants.
    pub fn to_envelope(&self) -> Value {
        match self {
            Self::Succeeded { item, output } => json!({
                "item": item,
                "status": "succeeded",
                "output": output,
                "error": Value::Null,
            }),
            Self::Failed { item, error } => json!({
                "item": item,
                "status": "failed",
                "output": Value::Null,
                "error": error,
            }),
        }
    }
}

/// The ledger of one iteration node: round index → settled outcome. Rounds settle in order, so
/// the map is contiguous `0..=k`; indexing by round keeps `entries` aligned with the input array
/// by construction (ADR D5).
pub type IterationLedger = BTreeMap<u32, RoundOutcome>;

/// Projects a complete ledger into the three exposed variables' values (ADR D3).
///
/// `entries` is the full ledger in input order, `output` keeps only succeeded rounds' collected
/// values in input order, and `failed_count` counts failed rounds. Types never depend on the
/// error strategy.
pub fn project_exposed_values(ledger: &IterationLedger) -> (Value, Value, Value) {
    let entries: Vec<Value> = ledger.values().map(RoundOutcome::to_envelope).collect();
    let output: Vec<Value> = ledger
        .values()
        .filter_map(|outcome| match outcome {
            RoundOutcome::Succeeded { output, .. } => Some(output.clone()),
            RoundOutcome::Failed { .. } => None,
        })
        .collect();
    let failed_count = ledger
        .values()
        .filter(|outcome| matches!(outcome, RoundOutcome::Failed { .. }))
        .count();
    (
        Value::Array(output),
        Value::Array(entries),
        json!(failed_count),
    )
}

/// Derives every composite region from `parentId` containment and enforces the six region
/// boundary rules (ADR "node runtime orchestration" D3, read per the iteration ADR's D3/D5).
///
/// The rules, in order: (1) the region is non-empty and fully entered from the owner's entry
/// edges; (2) no Output node inside; (3) no nested composite; (4) member out-edges stay inside
/// the region; (5) no outer node may target a member — the only way data or control enters a
/// region is the owner's own entry edges; (6) `maxIterations ≥ 1`, already enforced by config
/// parsing. Rule 5's enclosure keeps the outer projection from re-entering a region after the
/// composite completes, which would be a real cycle the DAG sort cannot see.
pub(super) fn derive_regions(
    nodes: &[(String, Option<String>, NodeType, bool)],
    edges: &[WorkflowGraphEdge],
) -> Result<HashMap<String, CompositeRegion>, GraphError> {
    // Membership first: parentId containment, validated to reference an existing composite node.
    let mut members_by_owner: HashMap<String, Vec<String>> = HashMap::new();
    for (node_id, parent_id, _, _) in nodes {
        let Some(parent) = parent_id else {
            continue;
        };
        let owner_kind = nodes
            .iter()
            .find(|(id, _, _, _)| id == parent)
            .map(|(_, _, kind, _)| *kind);
        match owner_kind {
            Some(NodeType::Iteration) => {
                members_by_owner
                    .entry(parent.clone())
                    .or_default()
                    .push(node_id.clone());
            }
            _ => {
                return Err(GraphError::InvalidRegion {
                    node_id: node_id.clone(),
                    reason: format!(
                        "parentId must reference an iteration node, but references {parent}"
                    ),
                });
            }
        }
    }

    let mut regions = HashMap::new();
    for (owner_id, member_ids) in members_by_owner {
        let region = CompositeRegion {
            owner_id: owner_id.clone(),
            member_ids,
        };
        validate_region(&region, nodes, edges)?;
        regions.insert(owner_id, region);
    }
    Ok(regions)
}

/// Applies the structural rules to one derived region.
fn validate_region(
    region: &CompositeRegion,
    nodes: &[(String, Option<String>, NodeType, bool)],
    edges: &[WorkflowGraphEdge],
) -> Result<(), GraphError> {
    let invalid = |node_id: &str, reason: String| GraphError::InvalidRegion {
        node_id: node_id.to_string(),
        reason,
    };
    let members: HashSet<&str> = region.member_ids.iter().map(String::as_str).collect();
    let kind_of = |node_id: &str| {
        nodes
            .iter()
            .find(|(id, _, _, _)| id == node_id)
            .map(|(_, _, kind, _)| *kind)
    };
    let interactive = |node_id: &str| {
        nodes
            .iter()
            .find(|(id, _, _, _)| id == node_id)
            .is_some_and(|(_, _, _, interactive)| *interactive)
    };

    // Rule 1: non-empty, and every member reachable from the owner's entry edges within the
    // region, so each round has a deterministic start set.
    if region.member_ids.is_empty() {
        return Err(invalid(
            &region.owner_id,
            "iteration region must contain at least one node".into(),
        ));
    }
    let mut reached: HashSet<&str> = HashSet::new();
    let mut frontier: Vec<&str> = Vec::new();
    for edge in edges {
        if edge.source == region.owner_id && members.contains(edge.target.as_str()) {
            frontier.push(&edge.target);
        }
    }
    if frontier.is_empty() {
        return Err(invalid(
            &region.owner_id,
            "iteration region must be entered by an edge from the iteration node".into(),
        ));
    }
    while let Some(current) = frontier.pop() {
        if !reached.insert(current) {
            continue;
        }
        for edge in edges {
            if edge.source == current && members.contains(edge.target.as_str()) {
                frontier.push(&edge.target);
            }
        }
    }
    for member in &region.member_ids {
        if !reached.contains(member.as_str()) {
            return Err(invalid(
                member,
                "iteration region node is not reachable from the iteration node's entry edge"
                    .into(),
            ));
        }
    }

    for member in &region.member_ids {
        // Rules 2 and 3: region content constraints. v1 also excludes interactive agent nodes
        // (ADR "iteration composite runtime" D5): an array of any length pausing on a human
        // each round needs batch-review UX that ships with the while-Loop ADR instead.
        match kind_of(member) {
            Some(NodeType::Output) => {
                return Err(invalid(
                    member,
                    "iteration region must not contain an output node".into(),
                ));
            }
            Some(NodeType::Iteration) => {
                return Err(invalid(
                    member,
                    "iteration region must not contain a nested iteration node".into(),
                ));
            }
            _ => {}
        }
        if interactive(member) {
            return Err(invalid(
                member,
                "iteration region must not contain an interactive node".into(),
            ));
        }
        for edge in edges {
            if edge.source != member.as_str() {
                continue;
            }
            // Rule 4: member out-edges stay inside the region.
            if !members.contains(edge.target.as_str()) {
                return Err(invalid(
                    member,
                    format!(
                        "iteration region node's edge to {} leaves the region; connect the iteration node's exit instead",
                        edge.target
                    ),
                ));
            }
        }
    }

    // Rule 5: only the owner and sibling members may target members; outer flow re-entering
    // the region after the composite completes would be a real cycle invisible to the DAG sort.
    for edge in edges {
        if edge.source != region.owner_id
            && !members.contains(edge.source.as_str())
            && members.contains(edge.target.as_str())
        {
            return Err(invalid(
                &edge.source,
                format!(
                    "edge targets {target} inside the iteration region; only the iteration node may enter its region",
                    target = edge.target
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn node(
        id: &str,
        kind: NodeType,
        parent: Option<&str>,
    ) -> (String, Option<String>, NodeType, bool) {
        (id.to_string(), parent.map(str::to_string), kind, false)
    }

    /** A node tuple flagged interactive, for the exclusion rule. */
    fn interactive_node(
        id: &str,
        kind: NodeType,
        parent: Option<&str>,
    ) -> (String, Option<String>, NodeType, bool) {
        (id.to_string(), parent.map(str::to_string), kind, true)
    }

    fn edge(source: &str, target: &str) -> WorkflowGraphEdge {
        WorkflowGraphEdge {
            source: source.to_string(),
            target: target.to_string(),
            source_handle: None,
        }
    }

    /// A minimal valid region: owner entered from outside, entry edge, closed interior.
    #[test]
    fn derives_a_valid_region_from_parent_id_containment() {
        let nodes = vec![
            node("start", NodeType::Start, None),
            node("iter", NodeType::Iteration, None),
            node("fix", NodeType::Agent, Some("iter")),
            node("out", NodeType::Output, None),
        ];
        let edges = vec![
            edge("start", "iter"),
            edge("iter", "fix"),
            edge("iter", "out"),
        ];
        let regions = derive_regions(&nodes, &edges).unwrap();
        assert_eq!(
            regions.get("iter").map(|region| region.member_ids.clone()),
            Some(vec!["fix".to_string()])
        );
    }

    /// Rule 1: a container with no entered member cannot start any round.
    #[test]
    fn rejects_a_region_without_entry_edges() {
        let nodes = vec![
            node("iter", NodeType::Iteration, None),
            node("fix", NodeType::Agent, Some("iter")),
        ];
        assert_eq!(
            derive_regions(&nodes, &[]).unwrap_err(),
            GraphError::InvalidRegion {
                node_id: "iter".to_string(),
                reason: "iteration region must be entered by an edge from the iteration node"
                    .to_string(),
            }
        );
    }

    /// Rule 1: a member not reachable from the entry edge is an orphan inside the region.
    #[test]
    fn rejects_a_member_unreachable_from_the_entry_edge() {
        let nodes = vec![
            node("iter", NodeType::Iteration, None),
            node("entry", NodeType::Agent, Some("iter")),
            node("orphan", NodeType::Agent, Some("iter")),
        ];
        let edges = vec![edge("iter", "entry")];
        assert_eq!(
            derive_regions(&nodes, &edges).unwrap_err(),
            GraphError::InvalidRegion {
                node_id: "orphan".to_string(),
                reason:
                    "iteration region node is not reachable from the iteration node's entry edge"
                        .to_string(),
            }
        );
    }

    /// Rules 2–4 each reject their violating shape with the offending node identified.
    #[test]
    fn rejects_output_nested_composite_and_escaping_edges() {
        // Output inside the region.
        let nodes = vec![
            node("iter", NodeType::Iteration, None),
            node("out", NodeType::Output, Some("iter")),
        ];
        let edges = vec![edge("iter", "out")];
        assert_eq!(
            derive_regions(&nodes, &edges).unwrap_err(),
            GraphError::InvalidRegion {
                node_id: "out".to_string(),
                reason: "iteration region must not contain an output node".to_string(),
            }
        );

        // Nested composite inside the region.
        let nodes = vec![
            node("iter", NodeType::Iteration, None),
            node("inner", NodeType::Iteration, Some("iter")),
        ];
        let edges = vec![edge("iter", "inner")];
        assert_eq!(
            derive_regions(&nodes, &edges).unwrap_err(),
            GraphError::InvalidRegion {
                node_id: "inner".to_string(),
                reason: "iteration region must not contain a nested iteration node".to_string(),
            }
        );

        // A member edge escaping the region.
        let nodes = vec![
            node("iter", NodeType::Iteration, None),
            node("fix", NodeType::Agent, Some("iter")),
            node("outer", NodeType::Agent, None),
            node("out", NodeType::Output, None),
        ];
        let edges = vec![
            edge("iter", "fix"),
            edge("fix", "outer"),
            edge("outer", "out"),
        ];
        assert_eq!(
            derive_regions(&nodes, &edges).unwrap_err(),
            GraphError::InvalidRegion {
                node_id: "fix".to_string(),
                reason: "iteration region node's edge to outer leaves the region; connect the iteration node's exit instead".to_string(),
            }
        );
    }

    /// Rule 5: an outer node targeting a member would re-enter the region after completion.
    #[test]
    fn rejects_outer_edges_targeting_members() {
        let nodes = vec![
            node("iter", NodeType::Iteration, None),
            node("fix", NodeType::Agent, Some("iter")),
            node("outer", NodeType::Agent, None),
        ];
        let edges = vec![edge("iter", "fix"), edge("outer", "fix")];
        assert_eq!(
            derive_regions(&nodes, &edges).unwrap_err(),
            GraphError::InvalidRegion {
                node_id: "outer".to_string(),
                reason: "edge targets fix inside the iteration region; only the iteration node may enter its region".to_string(),
            }
        );
    }

    /// An interactive agent inside the region is rejected (v1 exclusion, ADR D5).
    #[test]
    fn rejects_interactive_nodes_inside_the_region() {
        let nodes = vec![
            node("iter", NodeType::Iteration, None),
            interactive_node("review", NodeType::Agent, Some("iter")),
        ];
        let edges = vec![edge("iter", "review")];
        assert_eq!(
            derive_regions(&nodes, &edges).unwrap_err(),
            GraphError::InvalidRegion {
                node_id: "review".to_string(),
                reason: "iteration region must not contain an interactive node".to_string(),
            }
        );
    }

    /// parentId pointing at a non-iteration node is a containment error, not a region.
    #[test]
    fn rejects_parent_id_referencing_a_non_iteration_node() {
        let nodes = vec![
            node("agent", NodeType::Agent, None),
            node("fix", NodeType::Agent, Some("agent")),
        ];
        assert!(matches!(
            derive_regions(&nodes, &[]),
            Err(GraphError::InvalidRegion { .. })
        ));
    }

    /// The ledger projection keeps entries aligned with input order and filters `output` to
    /// succeeded rounds, regardless of strategy.
    #[test]
    fn projects_exposed_values_from_the_ledger() {
        let mut ledger = IterationLedger::new();
        ledger.insert(
            0,
            RoundOutcome::Succeeded {
                item: json!("a"),
                output: json!({ "approved": true }),
            },
        );
        ledger.insert(
            1,
            RoundOutcome::Failed {
                item: json!("b"),
                error: "collect target did not run this round".to_string(),
            },
        );
        ledger.insert(
            2,
            RoundOutcome::Succeeded {
                item: json!("c"),
                output: json!({ "approved": false }),
            },
        );
        let (output, entries, failed_count) = project_exposed_values(&ledger);
        assert_eq!(output, json!([{ "approved": true }, { "approved": false }]));
        assert_eq!(failed_count, json!(1));
        assert_eq!(
            entries,
            json!([
                { "item": "a", "status": "succeeded", "output": { "approved": true }, "error": null },
                { "item": "b", "status": "failed", "output": null, "error": "collect target did not run this round" },
                { "item": "c", "status": "succeeded", "output": { "approved": false }, "error": null },
            ])
        );
    }

    /// The envelope round-trips through serde so persisted ledgers stay readable.
    #[test]
    fn round_outcome_round_trips_through_serde() {
        let outcome = RoundOutcome::Failed {
            item: json!({"id": 7}),
            error: "boom".to_string(),
        };
        let encoded = serde_json::to_string(&outcome).unwrap();
        assert_eq!(
            serde_json::from_str::<RoundOutcome>(&encoded).unwrap(),
            outcome
        );
        // The wire tag is the snake_case status field.
        assert!(encoded.contains(r#""status":"failed""#));
    }

    /// Config parsing applies defaults and rejects illegal ceilings and nested collect paths.
    #[test]
    fn parses_iteration_config_with_defaults_and_validation() {
        let config: WireIterationConfig = serde_json::from_value(serde_json::json!({
            "iteratorSelector": ["start", "prs"],
            "collectSelector": ["fix", "output"]
        }))
        .unwrap();
        let parsed = config.into_model("iter").unwrap();
        assert_eq!(parsed.max_iterations, DEFAULT_MAX_ITERATIONS);
        assert_eq!(parsed.error_strategy, IterationErrorStrategy::Fail);

        let nested: WireIterationConfig = serde_json::from_value(serde_json::json!({
            "iteratorSelector": ["start", "prs"],
            "collectSelector": ["fix", "structured_output", "approved"]
        }))
        .unwrap();
        assert_eq!(
            nested.into_model("iter").unwrap_err(),
            GraphError::InvalidIteration {
                node_id: "iter".to_string(),
                reason: "collectSelector must reference a root variable without a nested path"
                    .to_string(),
            }
        );

        let zero: WireIterationConfig = serde_json::from_value(serde_json::json!({
            "iteratorSelector": ["start", "prs"],
            "collectSelector": ["fix", "output"],
            "maxIterations": 0
        }))
        .unwrap();
        assert_eq!(
            zero.into_model("iter").unwrap_err(),
            GraphError::InvalidIteration {
                node_id: "iter".to_string(),
                reason: "maxIterations must be at least 1, got 0".to_string(),
            }
        );
    }
}
