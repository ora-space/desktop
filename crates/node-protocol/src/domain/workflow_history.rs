//! Whole-run history is an explicit delivery variant, never a selected final session.
use crate::{ExecutionId, MessageValidationError, ObjectKey, StoredObject, is_canonical_uuid};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Cloud's ordered, immutable inventory of registered node attempts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowHistoryInput {
    pub workflow_run_id: String,
    pub sessions: Vec<WorkflowSessionAttempt>,
}

/// One registered execution, including attempts that never established a provider session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowSessionAttempt {
    pub execution_order: u32,
    pub node_id: String,
    pub attempt: u32,
    pub execution_id: ExecutionId,
    pub history_key: ObjectKey,
}

impl WorkflowHistoryInput {
    /// Rejects ambiguous identities, object collisions and reordered attempts before admission.
    pub(crate) fn validate(&self, manifest: &ObjectKey) -> Result<(), MessageValidationError> {
        let mut executions = HashSet::new();
        let mut attempts = HashSet::new();
        let mut keys = HashSet::from([manifest]);
        let mut previous = None;
        if !is_canonical_uuid(&self.workflow_run_id) || self.sessions.is_empty() {
            return Err(MessageValidationError::InvalidWorkflowHistory);
        }
        for session in &self.sessions {
            session.history_key.validate()?;
            let order = (session.execution_order, session.attempt);
            if session.node_id.is_empty()
                || session.attempt == 0
                || session.execution_id.is_empty()
                || !executions.insert(&session.execution_id)
                || !attempts.insert((&session.node_id, session.attempt))
                || !keys.insert(&session.history_key)
                || previous.is_some_and(|previous| previous >= order)
            {
                return Err(MessageValidationError::InvalidWorkflowHistory);
            }
            previous = Some(order);
        }
        Ok(())
    }
}

/// Versioned manifest stored at the delivery's history key; session files retain original bytes.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowHistoryManifest {
    pub schema_version: u32,
    pub workflow_run_id: String,
    pub checkout_execution_id: ExecutionId,
    pub sessions: Vec<WorkflowHistoryEntry>,
}

/// An entry repeats the frozen identity alongside its actual archival outcome.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct WorkflowHistoryEntry {
    pub execution_order: u32,
    pub node_id: String,
    pub attempt: u32,
    pub execution_id: ExecutionId,
    #[serde(flatten)]
    pub history: WorkflowSessionHistory,
}

impl<'de> Deserialize<'de> for WorkflowHistoryEntry {
    /// A strict wire record avoids serde's incompatible flatten/deny_unknown_fields combination.
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            execution_order: u32,
            node_id: String,
            attempt: u32,
            execution_id: ExecutionId,
            status: String,
            #[serde(default)]
            object: Option<StoredObject>,
        }
        let wire = Wire::deserialize(deserializer)?;
        let history = match (wire.status.as_str(), wire.object) {
            ("archived", Some(object)) => WorkflowSessionHistory::Archived { object },
            ("not_started", None) => WorkflowSessionHistory::NotStarted,
            ("unavailable", None) => WorkflowSessionHistory::Unavailable,
            _ => {
                return Err(serde::de::Error::custom(
                    "invalid workflow history status/object",
                ));
            }
        };
        Ok(Self {
            execution_order: wire.execution_order,
            node_id: wire.node_id,
            attempt: wire.attempt,
            execution_id: wire.execution_id,
            history,
        })
    }
}

/// Missing established history is a delivery failure, and cannot masquerade as never started.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum WorkflowSessionHistory {
    Archived { object: StoredObject },
    NotStarted,
    Unavailable,
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// Distinct executions and keys are necessary even when a failed node retries.
    #[test]
    fn inventory_rejects_aliases_and_reordering() {
        let manifest = ObjectKey::new("run/manifest.json");
        let first = WorkflowSessionAttempt {
            execution_order: 0,
            node_id: "agent".into(),
            attempt: 1,
            execution_id: ExecutionId::new("first"),
            history_key: ObjectKey::new("run/first.jsonl"),
        };
        let second = WorkflowSessionAttempt {
            execution_order: 1,
            attempt: 2,
            execution_id: ExecutionId::new("retry"),
            history_key: ObjectKey::new("run/retry.jsonl"),
            ..first.clone()
        };
        let input = WorkflowHistoryInput {
            workflow_run_id: "4c3b2a19-0f8e-4d7c-9b6a-5f4e3d2c1b0a".into(),
            sessions: vec![first.clone(), second.clone()],
        };
        assert_eq!(input.validate(&manifest), Ok(()));
        for sessions in [
            vec![first.clone(), first.clone()],
            vec![second.clone(), first.clone()],
            vec![
                first.clone(),
                WorkflowSessionAttempt {
                    history_key: manifest.clone(),
                    ..second.clone()
                },
            ],
            vec![
                first.clone(),
                WorkflowSessionAttempt {
                    execution_id: first.execution_id,
                    ..second
                },
            ],
        ] {
            assert_eq!(
                WorkflowHistoryInput {
                    sessions,
                    ..input.clone()
                }
                .validate(&manifest),
                Err(MessageValidationError::InvalidWorkflowHistory)
            );
        }
    }

    /// Manifest states retain the absence of a provider session without inventing an object.
    #[test]
    fn manifest_entries_round_trip_with_explicit_states() -> Result<(), serde_json::Error> {
        let entry = WorkflowHistoryEntry {
            execution_order: 0,
            node_id: "a".into(),
            attempt: 1,
            execution_id: ExecutionId::new("execution"),
            history: WorkflowSessionHistory::NotStarted,
        };
        let encoded = serde_json::to_value(&entry)?;
        assert_eq!(
            encoded,
            serde_json::json!({"execution_order":0,"node_id":"a","attempt":1,"execution_id":"execution","status":"not_started"})
        );
        assert_eq!(
            serde_json::from_value::<WorkflowHistoryEntry>(encoded)?,
            entry
        );
        Ok(())
    }
}
