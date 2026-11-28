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
    /// Withdraws committed projections together, so resuming and debugging a container
    /// cannot leave behind an old round decision or a settled item ledger.
    pub fn withdraw_writers(&mut self, writers: &std::collections::BTreeSet<String>) {
        let pool = &mut self.variable_pool;
        pool.values.retain(|key, _| {
            !pool
                .catalog
                .get(key)
                .is_some_and(|d| writers.contains(&d.writer))
                && !key
                    .strip_suffix(".selected_branch_id")
                    .is_some_and(|id| writers.contains(id))
        });
        pool.revision = pool.revision.saturating_add(1);
        self.condition_decisions
            .retain(|id, _| !writers.contains(id));
        self.iteration_ledger.retain(|id, _| !writers.contains(id));
        self.iteration_condition_decisions.retain(|key, _| {
            !writers.contains(key.rsplit_once('#').map_or(key.as_str(), |(id, _)| id))
        });
    }

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

    /// Rewinds only this container's effective ledger before a debug redispatch. Index zero
    /// starts a fresh batch. Missing prefix evidence must not silently shorten the result.
    pub fn rewind_iteration_ledger(
        &mut self,
        owner: &str,
        first_index: u32,
    ) -> Result<(), WorkflowRunPayloadError> {
        let ledger = self.iteration_ledger.get(owner);
        let prefix = ledger
            .into_iter()
            .flat_map(|rows| rows.range(..first_index));
        if !prefix.map(|(index, _)| *index).eq(0..first_index) {
            return Err(WorkflowRunPayloadError::MissingIterationPrefix {
                iteration_node_id: owner.to_owned(),
                first_index,
            });
        }
        if first_index == 0 {
            self.iteration_ledger.remove(owner);
        } else if let Some(rows) = self.iteration_ledger.get_mut(owner) {
            rows.split_off(&first_index);
        }
        Ok(())
    }

    /// Records one round's outcome once in the current effective ledger. An ordinary
    /// settlement replay cannot overwrite it; only an explicit rewind reopens the suffix.
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
    #[error("iteration {iteration_node_id} lacks a complete ledger prefix before {first_index}")]
    MissingIterationPrefix {
        iteration_node_id: String,
        first_index: u32,
    },
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

    /// Recomputing a failed item replaces its contribution without dropping a sibling ledger.
    #[test]
    fn rewind_iteration_ledger_preserves_verified_prefix_and_isolates_owner() {
        use super::super::iteration::project_exposed_values;
        use serde_json::json;

        let mut payload = WorkflowRunPayload::default();
        for index in 0..5 {
            let entry = RoundOutcome::Failed {
                item: json!(index),
                error: format!("old {index}"),
            };
            payload
                .record_round_outcome("batch", index, entry.clone())
                .unwrap();
            payload.record_round_outcome("other", index, entry).unwrap();
        }
        let before = payload.clone();
        for _ in 0..2 {
            payload.rewind_iteration_ledger("batch", 2).unwrap();
            for index in 2..5 {
                payload
                    .record_round_outcome(
                        "batch",
                        index,
                        RoundOutcome::Succeeded {
                            item: json!(index),
                            output: json!(index * 10),
                        },
                    )
                    .unwrap();
            }
            let (output, entries, failures) =
                project_exposed_values(&payload.iteration_ledger["batch"]);
            assert_eq!(output, json!([20, 30, 40]));
            assert_eq!(entries.as_array().unwrap().len(), 5);
            assert_eq!(failures, json!(2));
        }
        payload
            .iteration_ledger
            .get_mut("batch")
            .unwrap()
            .remove(&1);
        let incomplete = payload.clone();
        assert!(payload.rewind_iteration_ledger("batch", 2).is_err());
        assert_eq!(payload, incomplete);
        payload.rewind_iteration_ledger("batch", 0).unwrap();
        assert!(payload.iteration_ledger("batch").is_none());
        assert_eq!(
            payload.iteration_ledger("other"),
            before.iteration_ledger("other")
        );
    }

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
