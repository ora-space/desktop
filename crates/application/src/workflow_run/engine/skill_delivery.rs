use super::iteration::{IterationLedger, RoundOutcome};
use super::variable_pool::WorkflowVariablePool;
use ora_contracts::WorkflowRunLocale;
use ora_domain::AgentRef;
use ora_utils::path::StrictRelativePath;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Describes how an Agent accepts filesystem-delivered skill packages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSkillDelivery {
    /// The Agent cannot consume workflow-managed skills.
    Unsupported,
    /// The Agent discovers one Effect-materialized package below each declared workspace root.
    Filesystem {
        discovery_roots: SkillDiscoveryRoots,
    },
}

/// A non-empty, ordered set of worktree-relative roots an Agent scans for skill packages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDiscoveryRoots(Vec<StrictRelativePath>);

impl SkillDiscoveryRoots {
    /// Creates a capability with one required root followed by any additional discovery roots.
    pub fn new(first: StrictRelativePath, additional: Vec<StrictRelativePath>) -> Self {
        let mut roots = vec![first];
        for root in additional {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
        Self(roots)
    }

    /// Iterates over the stable, de-duplicated discovery-root order.
    pub fn iter(&self) -> impl Iterator<Item = &StrictRelativePath> {
        self.0.iter()
    }
}

/// Reports why an Agent's frozen skill-delivery capability could not be obtained.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AgentSkillDeliveryError {
    #[error("agent skill-delivery capability is unavailable")]
    Unavailable,
    #[error("agent skill-delivery capability is invalid: {message}")]
    Invalid { message: String },
}

/// Resolves the skill-delivery capability declared by an Agent provider.
///
/// Implementations are expected to return a stable capability snapshot suitable for freezing into
/// a workflow run. Plugin-backed implementations should read a previously validated capability
/// registry rather than contacting a live plugin during worktree creation.
pub trait AgentSkillDeliveryProvider: Send + Sync {
    /// Returns how the named Agent expects skill packages to be placed in its session worktree.
    fn skill_delivery(
        &self,
        agent_ref: &AgentRef,
    ) -> Result<AgentSkillDelivery, AgentSkillDeliveryError>;
}

/// Records the actual skill packages made available to one frozen workflow run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillMaterializationReceipt {
    pub bindings: Vec<MaterializedSkillBinding>,
}

impl SkillMaterializationReceipt {
    /// Returns the frozen skill bindings belonging to one graph node in declaration order.
    pub fn bindings_for_node(&self, node_id: &str) -> Vec<&MaterializedSkillBinding> {
        self.bindings
            .iter()
            .filter(|binding| binding.node_id == node_id)
            .collect()
    }
}

/// Binds one graph node's declared skill to its executable name and actual package locations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterializedSkillBinding {
    pub node_id: String,
    pub skill_id: String,
    pub invocation_name: String,
    pub package_paths: Vec<StrictRelativePath>,
}

impl MaterializedSkillBinding {
    /// Resolves every frozen package path beneath the run worktree for prompt presentation.
    pub fn absolute_package_paths(&self, worktree_root: &Path) -> Vec<PathBuf> {
        self.package_paths
            .iter()
            .map(|path| path.to_path(worktree_root))
            .collect()
    }
}

/// Internal payload frozen with a workflow run at creation time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunPayload {
    pub locale: WorkflowRunLocale,
    pub skill_materialization: SkillMaterializationReceipt,
    /// Identifies the owner of editable Start variables without treating the run instruction as one.
    #[serde(default)]
    pub start_node_id: Option<String>,
    #[serde(default)]
    pub variable_pool: WorkflowVariablePool,
    /// Internal Condition routing decisions kept outside the user-selectable variable pool.
    #[serde(default)]
    pub condition_decisions: BTreeMap<String, String>,
    /// Per-round settled outcomes of each iteration node, keyed by iteration node id then round
    /// index (ADR "iteration composite runtime" D5). Old payloads parse without it.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub iteration_ledger: BTreeMap<String, IterationLedger>,
    /// Per-round Condition decisions inside iteration regions, keyed by
    /// `{condition_node_id}#{round}`. Condition node ids are unique per graph and each belongs
    /// to at most one region, so the pair identifies the decision without ambiguity while
    /// keeping a single flat map for `#[serde(default)]` compatibility.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub iteration_condition_decisions: BTreeMap<String, String>,
    /// When true, a re-dispatched node receives a short prompt block describing its last
    /// agent-behaviour failure. Older payloads without this key deserialize as on.
    #[serde(default = "default_inject_last_failure")]
    pub inject_last_failure: bool,
}

/// Older run payloads omitted this switch; treating them as on preserves the current default.
fn default_inject_last_failure() -> bool {
    true
}

impl Default for WorkflowRunPayload {
    /// A payload for runs persisted before typed payloads existed; the locale defaults to
    /// English, matching how pre-locale runs were rendered.
    fn default() -> Self {
        Self::new(WorkflowRunLocale::EnUs, Default::default())
    }
}

impl WorkflowRunPayload {
    /// Creates the immutable execution metadata captured while the run worktree is initialized.
    pub fn new(
        locale: WorkflowRunLocale,
        skill_materialization: SkillMaterializationReceipt,
    ) -> Self {
        Self {
            locale,
            skill_materialization,
            start_node_id: None,
            variable_pool: WorkflowVariablePool::default(),
            condition_decisions: BTreeMap::new(),
            iteration_ledger: BTreeMap::new(),
            iteration_condition_decisions: BTreeMap::new(),
            inject_last_failure: true,
        }
    }

    /// Creates run metadata with the graph-derived variable pool used by a new execution.
    pub fn with_variable_pool(
        locale: WorkflowRunLocale,
        skill_materialization: SkillMaterializationReceipt,
        start_node_id: Option<String>,
        variable_pool: WorkflowVariablePool,
    ) -> Self {
        Self {
            locale,
            skill_materialization,
            start_node_id,
            variable_pool,
            condition_decisions: BTreeMap::new(),
            iteration_ledger: BTreeMap::new(),
            iteration_condition_decisions: BTreeMap::new(),
            inject_last_failure: true,
        }
    }

    /// Sets whether a later attempt of a failed node should see the previous failure in its prompt.
    pub fn with_inject_last_failure(mut self, inject: bool) -> Self {
        self.inject_last_failure = inject;
        self
    }

    /// Returns Condition routing state while migrating decisions stored by older payloads.
    pub fn resolved_condition_decisions(&self) -> BTreeMap<String, String> {
        let mut decisions = self
            .variable_pool
            .values
            .iter()
            .filter_map(|(selector, value)| {
                let node_id = selector.strip_suffix(".selected_branch_id")?;
                Some((node_id.to_string(), value.as_str()?.to_string()))
            })
            .collect::<BTreeMap<_, _>>();
        // The dedicated field is authoritative when a payload contains both old and new state.
        decisions.extend(self.condition_decisions.clone());
        decisions
    }

    /// Returns the Condition decisions active in one round of an iteration region, keyed by
    /// condition node id. The iteration projection consumes only the current round's decisions
    /// (ADR "iteration composite runtime" D5).
    pub fn iteration_round_decisions(&self, round: u32) -> BTreeMap<String, String> {
        let suffix = format!("#{round}");
        self.iteration_condition_decisions
            .iter()
            .filter_map(|(key, branch)| {
                let condition_id = key.strip_suffix(&suffix)?;
                Some((condition_id.to_string(), branch.clone()))
            })
            .collect()
    }

    /// Returns the settled ledger of one iteration node, if any rounds have settled.
    pub fn iteration_ledger(&self, iteration_node_id: &str) -> Option<&IterationLedger> {
        self.iteration_ledger.get(iteration_node_id)
    }

    /// Builds the per-round Condition decision key for a condition node inside a region.
    pub fn iteration_decision_key(condition_node_id: &str, round: u32) -> String {
        format!("{condition_node_id}#{round}")
    }

    /// Records one round's settled outcome for an iteration node. Entries are append-only per
    /// run: recording an already-settled round is rejected so ledger history stays immutable.
    pub fn record_round_outcome(
        &mut self,
        iteration_node_id: &str,
        round: u32,
        outcome: RoundOutcome,
    ) -> Result<(), WorkflowRunPayloadError> {
        let ledger = self
            .iteration_ledger
            .entry(iteration_node_id.to_string())
            .or_default();
        if ledger.contains_key(&round) {
            return Err(WorkflowRunPayloadError::RoundAlreadySettled {
                iteration_node_id: iteration_node_id.to_string(),
                round,
            });
        }
        ledger.insert(round, outcome);
        Ok(())
    }
}

/// Failures raised while mutating private run payload state.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkflowRunPayloadError {
    #[error("iteration {iteration_node_id} round {round} is already settled")]
    RoundAlreadySettled {
        iteration_node_id: String,
        round: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Persisted receipts round-trip without losing normalized placement paths.
    #[test]
    fn workflow_run_payload_round_trips_materialized_skill_bindings() {
        let expected = WorkflowRunPayload::new(
            WorkflowRunLocale::EnUs,
            SkillMaterializationReceipt {
                bindings: vec![MaterializedSkillBinding {
                    node_id: "review".to_string(),
                    skill_id: "catalog-id".to_string(),
                    invocation_name: "review".to_string(),
                    package_paths: vec![StrictRelativePath::parse(".agent/skills/review").unwrap()],
                }],
            },
        );

        let encoded = serde_json::to_string(&expected).unwrap();
        assert_eq!(
            serde_json::from_str::<WorkflowRunPayload>(&encoded).unwrap(),
            expected
        );
    }

    /// Deserialization revalidates receipt paths so persisted traversal cannot escape a worktree.
    #[test]
    fn workflow_run_payload_rejects_unsafe_materialization_paths() {
        let encoded = r#"{"locale":"en-US","skillMaterialization":{"bindings":[{"nodeId":"review","skillId":"catalog-id","invocationName":"review","packagePaths":["../escape"]}]}}"#;

        assert!(serde_json::from_str::<WorkflowRunPayload>(encoded).is_err());
    }

    /// Legacy branch variables remain restart-readable but dedicated decisions take precedence.
    #[test]
    fn workflow_run_payload_resolves_legacy_condition_decisions_privately() {
        let mut payload = WorkflowRunPayload::new(WorkflowRunLocale::EnUs, Default::default());
        payload
            .variable_pool
            .declare("condition-1.selected_branch_id", "string", "condition-1");
        payload
            .variable_pool
            .set(
                "condition-1.selected_branch_id",
                "condition-1",
                serde_json::json!("legacy-case"),
            )
            .unwrap();
        payload
            .condition_decisions
            .insert("condition-1".to_string(), "current-case".to_string());

        assert_eq!(
            payload.resolved_condition_decisions(),
            BTreeMap::from([("condition-1".to_string(), "current-case".to_string())])
        );
    }

    /// Payloads written before this field existed still inject last-failure context by default.
    #[test]
    fn workflow_run_payload_defaults_inject_last_failure_to_true() {
        let payload = WorkflowRunPayload::new(WorkflowRunLocale::EnUs, Default::default());
        let mut value = serde_json::to_value(&payload).unwrap();
        value
            .as_object_mut()
            .expect("payload object")
            .remove("injectLastFailure");
        let decoded: WorkflowRunPayload = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.inject_last_failure, true);

        let off = WorkflowRunPayload::new(WorkflowRunLocale::EnUs, Default::default())
            .with_inject_last_failure(false);
        let round_tripped: WorkflowRunPayload =
            serde_json::from_str(&serde_json::to_string(&off).unwrap()).unwrap();
        assert_eq!(round_tripped, off);
        assert_eq!(round_tripped.inject_last_failure, false);
    }
}
