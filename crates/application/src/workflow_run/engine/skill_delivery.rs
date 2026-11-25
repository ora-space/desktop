use super::iteration::{IterationLedger, RoundOutcome};
use super::variable_pool::WorkflowVariablePool;
use ora_contracts::WorkflowRunLocale;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Records the frozen catalog skill bindings selected for one workflow run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowSkillReceipt {
    pub bindings: Vec<FrozenSkillBinding>,
}

impl WorkflowSkillReceipt {
    /// Returns the frozen skill bindings belonging to one graph node in declaration order.
    pub fn bindings_for_node(&self, node_id: &str) -> Vec<&FrozenSkillBinding> {
        self.bindings
            .iter()
            .filter(|binding| binding.node_id == node_id)
            .collect()
    }
}

/// Binds one graph node's declared skill to its frozen catalog identifier and invocation name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrozenSkillBinding {
    pub node_id: String,
    pub skill_id: String,
    /// Exact catalog row identifier resolved when the run or snapshot was frozen.
    ///
    /// Legacy pre-Effect-retirement receipts deserialize with `None` and fall back to resolving
    /// `skill_id`; every newly frozen receipt records the concrete catalog ID so dispatch and
    /// prechecks resolve strictly by ID and never switch to another same-named skill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_skill_id: Option<String>,
    pub invocation_name: String,
}

/// Internal payload frozen with a workflow run at creation time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunPayload {
    pub locale: WorkflowRunLocale,
    /// Preserves the persisted `"skillMaterialization"` JSON key for compatibility with existing rows.
    #[serde(rename = "skillMaterialization")]
    pub skill_receipt: WorkflowSkillReceipt,
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
    /// Creates the immutable execution metadata captured while the run is initialized.
    pub fn new(locale: WorkflowRunLocale, skill_receipt: WorkflowSkillReceipt) -> Self {
        Self {
            locale,
            skill_receipt,
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
        skill_receipt: WorkflowSkillReceipt,
        start_node_id: Option<String>,
        variable_pool: WorkflowVariablePool,
    ) -> Self {
        Self {
            locale,
            skill_receipt,
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

    /// Persisted skill bindings round-trip with catalog id and invocation name.
    #[test]
    fn workflow_run_payload_round_trips_frozen_skill_bindings() {
        let expected = WorkflowRunPayload::new(
            WorkflowRunLocale::EnUs,
            WorkflowSkillReceipt {
                bindings: vec![FrozenSkillBinding {
                    node_id: "review".to_string(),
                    skill_id: "review".to_string(),
                    catalog_skill_id: Some("plugin:official/review-pack:review".to_string()),
                    invocation_name: "review".to_string(),
                }],
            },
        );

        let encoded = serde_json::to_string(&expected).unwrap();
        assert_eq!(
            serde_json::from_str::<WorkflowRunPayload>(&encoded).unwrap(),
            expected
        );
    }

    /// Legacy payloads recorded before Effect retirement still carry `packagePaths` and omit
    /// `catalogSkillId`; ignore `packagePaths` and default `catalog_skill_id` to `None`.
    #[test]
    fn workflow_run_payload_ignores_legacy_package_paths() {
        let encoded = r#"{"locale":"en-US","skillMaterialization":{"bindings":[{"nodeId":"review","skillId":"catalog-id","invocationName":"review","packagePaths":[".claude/skills/review"]}]}}"#;

        assert_eq!(
            serde_json::from_str::<WorkflowRunPayload>(encoded).unwrap(),
            WorkflowRunPayload::new(
                WorkflowRunLocale::EnUs,
                WorkflowSkillReceipt {
                    bindings: vec![FrozenSkillBinding {
                        node_id: "review".to_string(),
                        skill_id: "catalog-id".to_string(),
                        catalog_skill_id: None,
                        invocation_name: "review".to_string(),
                    }],
                },
            )
        );
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
