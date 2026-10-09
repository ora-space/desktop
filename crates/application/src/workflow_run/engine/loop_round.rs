//! Pure round decisions, computed before a persistence adapter commits any advancement.

use super::condition::{ConditionConfig, ConditionError, ELSE_BRANCH_ID, evaluate_condition};
use super::graph::WorkflowGraph;
use super::loop_config::{LoopConfig, LoopInitialValue};
use super::variable_pool::{VariableSelector, WorkflowVariablePool, WorkflowVariablePoolError};
use super::variable_value::normalize_workflow_value;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use thiserror::Error;

/// A completed round either supplies the next round's inputs or exports the final results.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopRoundDecision {
    Continue { carried: BTreeMap<String, Value> },
    Succeeded { outputs: BTreeMap<String, Value> },
}

/// Durable execution data private to one round scope.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopRoundExecutionState {
    pub variable_pool: WorkflowVariablePool,
    pub condition_decisions: BTreeMap<String, String>,
    /// A committed break freezes outputs while the session owner drains cancelled workers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<LoopExitState>,
}

/// Immutable exit facts survive crashes between cancellation and loop completion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopExitState {
    pub node_run_id: String,
    pub requested_at: i64,
    pub result: Result<BTreeMap<String, Value>, String>,
}

/// Failures which must abort advancement without publishing a partially updated variable set.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LoopRoundError {
    #[error("Loop container does not exist: {node_id}")]
    UnknownLoop { node_id: String },
    #[error("carried values do not match the Loop variable declarations")]
    InvalidCarriedVariables,
    #[error("invalid Loop round {round}; expected 1 through {max_iterations}")]
    InvalidRound { round: u32, max_iterations: u32 },
    /// `observed` is a short reading of the until-condition inputs from the last round.
    /// The match itself stays exact; the text only tells a person whether the model
    /// missed the token or the loop logic never became true.
    #[error("Loop did not terminate within {max_iterations} rounds; last observed {observed}")]
    LimitReached {
        max_iterations: u32,
        observed: String,
    },
    #[error("Loop variable {name} does not match declared type {value_type}")]
    TypeMismatch { name: String, value_type: String },
    #[error("Loop selector has no value in this round: {selector}")]
    UnsetValue { selector: String },
    #[error(transparent)]
    Variable(#[from] WorkflowVariablePoolError),
    #[error(transparent)]
    Condition(#[from] ConditionError),
}

impl WorkflowGraph {
    /// Builds a fresh round pool; only upstream outer values and explicit carried values are imported.
    pub fn loop_round_pool(
        &self,
        loop_id: &str,
        outer: &WorkflowVariablePool,
        carried: &BTreeMap<String, Value>,
    ) -> Result<WorkflowVariablePool, LoopRoundError> {
        let (config, body) =
            self.loop_body(loop_id)
                .ok_or_else(|| LoopRoundError::UnknownLoop {
                    node_id: loop_id.into(),
                })?;
        if carried.len() != config.variables.len()
            || config
                .variables
                .iter()
                .any(|variable| !carried.contains_key(&variable.name))
        {
            return Err(LoopRoundError::InvalidCarriedVariables);
        }
        let ancestors: std::collections::HashSet<_> = self
            .transitive_predecessors(loop_id)
            .into_iter()
            .map(|node| node.id.as_str())
            .collect();
        let globals: std::collections::HashSet<_> = self
            .global_variables()
            .iter()
            .map(|variable| variable.name.as_str())
            .collect();
        let mut pool = WorkflowVariablePool::from_graph(body);
        // Never clone the previous round's pool: absent branch outputs must remain unassigned.
        // Imported definitions retain their original writer, so child nodes cannot mutate them.
        for (key, definition) in &outer.catalog {
            if !ancestors.contains(definition.writer.as_str()) && !globals.contains(key.as_str()) {
                continue;
            }
            pool.catalog.insert(key.clone(), definition.clone());
            if let Some(value) = outer.values.get(key) {
                pool.values.insert(key.clone(), value.clone());
            }
        }
        for variable in &config.variables {
            let key = format!("{loop_id}.{}", variable.name);
            pool.declare(&key, &variable.value_type, loop_id);
            pool.set(&key, loop_id, carried[&variable.name].clone())?;
        }
        Ok(pool)
    }
}

impl LoopConfig {
    /// Freezes carried inputs once at container entry, without modifying the outer pool.
    pub fn initialize_carried(
        &self,
        outer: &WorkflowVariablePool,
    ) -> Result<BTreeMap<String, Value>, LoopRoundError> {
        self.variables
            .iter()
            .map(|variable| {
                let value = match &variable.initial {
                    LoopInitialValue::Constant(value) => value.clone(),
                    LoopInitialValue::Variable(selector) => resolve_required(outer, selector)?,
                };
                let value =
                    normalize_workflow_value(value, &variable.value_type).ok_or_else(|| {
                        LoopRoundError::TypeMismatch {
                            name: variable.name.clone(),
                            value_type: variable.value_type.clone(),
                        }
                    })?;
                Ok((variable.name.clone(), value))
            })
            .collect()
    }

    /// Resolves only the public result when exiting; no next-round feedback is required.
    pub fn exit_outputs(
        &self,
        pool: &WorkflowVariablePool,
    ) -> Result<BTreeMap<String, Value>, LoopRoundError> {
        self.outputs
            .iter()
            .map(|output| {
                Ok((
                    output.name.clone(),
                    resolve_required(pool, &output.variable_selector)?,
                ))
            })
            .collect()
    }

    /// Decides advancement from one immutable completed-round snapshot using one-based rounds.
    pub fn complete_round(
        &self,
        round: u32,
        completed: &WorkflowVariablePool,
    ) -> Result<LoopRoundDecision, LoopRoundError> {
        if round == 0 || round > self.max_iterations {
            return Err(LoopRoundError::InvalidRound {
                round,
                max_iterations: self.max_iterations,
            });
        }
        // All feedback reads observe the same snapshot, including assignments that swap values.
        // Returning owned values lets the repository commit them together or discard them all.
        let carried = self
            .variables
            .iter()
            .map(|variable| {
                let value = resolve_required(completed, &variable.feedback)?;
                let value =
                    normalize_workflow_value(value, &variable.value_type).ok_or_else(|| {
                        LoopRoundError::TypeMismatch {
                            name: variable.name.clone(),
                            value_type: variable.value_type.clone(),
                        }
                    })?;
                Ok((variable.name.clone(), value))
            })
            .collect::<Result<BTreeMap<_, _>, LoopRoundError>>()?;
        if self
            .until
            .cases
            .iter()
            .any(|case| !case.conditions.is_empty())
            && evaluate_condition(&self.until, completed)? != ELSE_BRANCH_ID
        {
            let outputs = self.exit_outputs(completed)?;
            return Ok(LoopRoundDecision::Succeeded { outputs });
        }
        // A successful final permitted round still succeeds; only a false termination hits the cap.
        if round == self.max_iterations {
            return Err(LoopRoundError::LimitReached {
                max_iterations: self.max_iterations,
                observed: observed_until_summary(&self.until, completed),
            });
        }
        Ok(LoopRoundDecision::Continue { carried })
    }
}

/// One line a person can read on the failed run. Full model replies are not stored here.
const OBSERVED_VALUE_LIMIT: usize = 160;

/// Reads every until-condition input from the round that just failed the cap.
fn observed_until_summary(until: &ConditionConfig, pool: &WorkflowVariablePool) -> String {
    let mut parts = Vec::new();
    for case in &until.cases {
        for rule in &case.conditions {
            let label = selector_label(&rule.variable_selector);
            let shown = match pool.resolve(&rule.variable_selector) {
                Ok(Some(value)) => summarize_observed_value(value),
                Ok(None) => "unset".to_string(),
                Err(_) => "unavailable".to_string(),
            };
            parts.push(format!("{label}={shown}"));
        }
    }
    if parts.is_empty() {
        "no until comparison".to_string()
    } else {
        parts.join(", ")
    }
}

/// `{node.root}` plus any nested object path the condition actually compared.
fn selector_label(selector: &super::variable_pool::VariableSelector) -> String {
    let mut label = selector.qualified();
    for segment in &selector.nested {
        label.push('.');
        label.push_str(segment);
    }
    label
}

/// Collapses whitespace and keeps the error line short enough to show in the run inspector.
fn summarize_observed_value(value: &Value) -> String {
    let raw = match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= OBSERVED_VALUE_LIMIT {
        return collapsed;
    }
    let truncated: String = collapsed.chars().take(OBSERVED_VALUE_LIMIT).collect();
    format!("{truncated}…")
}

/// An inactive branch's unset value is an error, never a previous round's implicit fallback.
fn resolve_required(
    pool: &WorkflowVariablePool,
    selector: &VariableSelector,
) -> Result<Value, LoopRoundError> {
    pool.resolve(selector)?
        .cloned()
        .ok_or_else(|| LoopRoundError::UnsetValue {
            selector: std::iter::once(selector.qualified())
                .chain(selector.nested.iter().cloned())
                .collect::<Vec<_>>()
                .join("."),
        })
}

#[cfg(test)]
mod tests;
