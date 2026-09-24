//! Automatic retry of failed agent attempts.
//!
//! An agent node's `agentConfig.retry` policy decides whether a failed attempt is replaced by a
//! new attempt after a backoff wait instead of failing the node. The engine owns the decision
//! (`engine/retry_scheduler.rs`); this module holds the policy, its wire validation, the
//! persisted payload shapes the UI reads, and the persistence and timer ports the decision is
//! executed through.
//!
//! A waiting attempt is an ordinary live node-run row in the `Running` status whose payload
//! carries a [`NodeRetryWait`] under [`RETRY_WAIT_KEY`] and has no `started_at`. Keeping it
//! `Running` is what makes every existing rule treat it as in flight: the scheduler never
//! dispatches the node again or its dependents, the run is not drained, cancel settles it, and
//! the boot sweep interrupts or absorbs it exactly like a generating row.

use super::failure::{NodeFailure, NodeFailureKind};
use super::graph::GraphError;
use crate::RepositoryError;
use ora_domain::{WorkflowNodeRun, WorkflowNodeRunId, WorkflowRunId};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Retries an agent node gets when its graph declares no `agentConfig.retry`.
pub const DEFAULT_MAX_RETRIES: u32 = 2;
/// Wait before the first retry when the graph declares no `agentConfig.retry`.
pub const DEFAULT_INITIAL_DELAY_SECONDS: u32 = 10;
/// Upper bound accepted for `agentConfig.retry.maxRetries`.
pub const MAX_RETRIES_LIMIT: u32 = 5;
/// Upper bound accepted for `agentConfig.retry.initialDelaySeconds`.
pub const INITIAL_DELAY_LIMIT_SECONDS: u32 = 300;
/// Ceiling of any single backoff wait, whatever the policy and retry number.
pub const MAX_RETRY_WAIT_SECONDS: u64 = 600;

/// Node-run payload key of a waiting attempt's [`NodeRetryWait`].
pub const RETRY_WAIT_KEY: &str = "retry_wait";
/// Node-run payload key of an automatic attempt's [`NodeAutoRetry`].
pub const AUTO_RETRY_KEY: &str = "auto_retry";
/// Node-run payload key listing the earlier attempts of an automatic-retry chain; see
/// [`retry_chain_from_payload`].
pub const RETRY_CHAIN_KEY: &str = "retry_chain";

/// Reads `payload.retry_chain` of an attempt an automatic retry started: the node-run ids of the
/// earlier attempts of the same chain (same node and round, since the last start, restart, or
/// resume), oldest first. Empty for any other row.
///
/// A resume after the chain is exhausted treats the chain as one unit: rollback restores the
/// worktree from the first attempt's checkpoint and covers the files every attempt changed.
pub fn retry_chain_from_payload(payload: Option<&str>) -> Vec<String> {
    payload
        .and_then(|payload| serde_json::from_str::<Map<String, Value>>(payload).ok())
        .and_then(|mut payload| payload.remove(RETRY_CHAIN_KEY))
        .and_then(|chain| serde_json::from_value(chain).ok())
        .unwrap_or_default()
}

/// The automatic retry policy of one agent node (`agentConfig.retry`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentRetryPolicy {
    pub enabled: bool,
    /// Retries after the first attempt of one budget; `0` disables retrying.
    pub max_retries: u32,
    /// Wait before the first retry; each later retry doubles it, capped at
    /// [`MAX_RETRY_WAIT_SECONDS`].
    pub initial_delay_seconds: u32,
}

impl Default for AgentRetryPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: DEFAULT_MAX_RETRIES,
            initial_delay_seconds: DEFAULT_INITIAL_DELAY_SECONDS,
        }
    }
}

impl AgentRetryPolicy {
    /// The wait before retry number `retry` (1-based): `initial × 2^(retry-1)`, capped at
    /// [`MAX_RETRY_WAIT_SECONDS`].
    pub fn wait_seconds(&self, retry: u32) -> u64 {
        let factor = 1_u64
            .checked_shl(retry.saturating_sub(1))
            .unwrap_or(u64::MAX);
        u64::from(self.initial_delay_seconds)
            .saturating_mul(factor)
            .min(MAX_RETRY_WAIT_SECONDS)
    }

    /// The retry number that replaces a failed attempt of `kind` after `retries_used` retries
    /// of the current budget, or `None` when the failure must stand.
    pub fn next_retry(&self, kind: NodeFailureKind, retries_used: u32) -> Option<u32> {
        (self.enabled && kind.auto_retry() && retries_used < self.max_retries)
            .then(|| retries_used.saturating_add(1))
    }
}

/// Validates `agentConfig.retry` of node `node_id`.
///
/// A missing (or `null`) policy means the default. A present policy must be an object carrying
/// all three fields, so an editor bug cannot silently fall back to a policy the author never saw.
/// Unknown keys are ignored like everywhere else in the graph document.
pub(super) fn parse_retry_policy(
    node_id: &str,
    retry: Option<&Value>,
) -> Result<AgentRetryPolicy, GraphError> {
    let invalid = |reason: String| GraphError::InvalidRetry {
        node_id: node_id.to_string(),
        reason,
    };
    let object = match retry {
        None | Some(Value::Null) => return Ok(AgentRetryPolicy::default()),
        Some(Value::Object(object)) => object,
        Some(other) => return Err(invalid(format!("retry must be an object, got {other}"))),
    };
    let enabled = match object.get("enabled") {
        Some(Value::Bool(enabled)) => *enabled,
        Some(other) => {
            return Err(invalid(format!(
                "retry.enabled must be a boolean, got {other}"
            )));
        }
        None => return Err(invalid("retry.enabled is required".to_string())),
    };
    Ok(AgentRetryPolicy {
        enabled,
        max_retries: bounded_integer(object, "maxRetries", MAX_RETRIES_LIMIT).map_err(invalid)?,
        initial_delay_seconds: bounded_integer(
            object,
            "initialDelaySeconds",
            INITIAL_DELAY_LIMIT_SECONDS,
        )
        .map_err(invalid)?,
    })
}

/// Reads one required retry field as an integer in `0..=max`, or explains why it is invalid.
fn bounded_integer(object: &Map<String, Value>, field: &str, max: u32) -> Result<u32, String> {
    let value = object
        .get(field)
        .ok_or_else(|| format!("retry.{field} is required"))?;
    value
        .as_u64()
        .and_then(|number| u32::try_from(number).ok())
        .filter(|number| *number <= max)
        .ok_or_else(|| format!("retry.{field} must be an integer from 0 to {max}, got {value}"))
}

/// Persisted under `payload.retry_wait` of an attempt that waits for its backoff to elapse.
///
/// The key is removed when the attempt starts or is settled, so a live `Running` row carrying it
/// is exactly a waiting attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRetryWait {
    /// Attempt number this row will run as (same numbering as `error_detail.attempt`).
    pub attempt: u32,
    /// Last attempt number the current retry budget allows.
    pub max_attempt: u32,
    /// Retry number within the current budget, 1-based.
    pub retry: u32,
    /// `maxRetries` of the policy the retry was scheduled under.
    pub max_retries: u32,
    /// Backoff wait in milliseconds.
    pub delay_ms: i64,
    /// Unix millis when the failed attempt was replaced by this row.
    pub scheduled_at: i64,
    /// Unix millis at which the attempt starts.
    pub due_at: i64,
    /// The failed attempt this row replaces (now soft-deleted).
    pub previous_node_run_id: String,
}

/// Persisted under `payload.auto_retry` of every attempt started by an automatic retry.
///
/// It survives the attempt's own failure, which is how the engine counts the retries already
/// used; rows started by the scheduler or by a manual resume carry none, so the budget resets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeAutoRetry {
    pub retry: u32,
    pub max_retries: u32,
}

impl NodeAutoRetry {
    /// Reads the marker of a node-run payload; absent or unreadable means a first attempt.
    pub fn from_payload(payload: Option<&str>) -> Option<Self> {
        let mut payload: Map<String, Value> = serde_json::from_str(payload?).ok()?;
        serde_json::from_value(payload.remove(AUTO_RETRY_KEY)?).ok()
    }
}

/// A retry the engine schedules in place of one failed attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRetryToSchedule {
    /// Id of the new waiting row.
    pub node_run_id: WorkflowNodeRunId,
    pub retry: u32,
    pub max_retries: u32,
    pub delay_ms: i64,
    pub due_at: i64,
}

/// Outcome of scheduling one retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleNodeRetryResult {
    /// The failed attempt is recorded and soft-deleted, and the waiting row is live.
    Scheduled,
    /// The run is no longer `Running`; nothing changed, so the failure takes the normal path.
    RunNotActive,
    /// The attempt is no longer `Running` (a late or duplicate callback); nothing changed.
    NotRunning,
    NotFound,
}

/// Outcome of waking one waiting attempt.
#[derive(Debug, Clone, PartialEq)]
pub enum BeginNodeRetryResult {
    /// The wait elapsed: the row is now a started attempt and must be dispatched.
    Started(Box<WorkflowNodeRun>),
    /// The wake came early (for example after a wall-clock change); wake again at `due_at`.
    NotDue {
        due_at: i64,
    },
    /// The run left `Running` without settling the wait; the row is now `Cancelled`.
    Abandoned,
    /// The row is not a waiting attempt any more (started, cancelled, failed, interrupted).
    NotWaiting,
    NotFound,
}

/// Persistence of automatic retries. Every method is one immediate transaction.
pub trait WorkflowRetryRepository {
    /// Replaces the `Running` attempt `failed_node_run_id` by a waiting attempt: records the
    /// failure on the attempt exactly as `fail_node` would (`payload.error_detail`, attempt
    /// number), soft-deletes it, and inserts the waiting row for the same node, scope, and round.
    /// The run and its `current_nodes` anchor are unchanged.
    fn schedule_node_retry(
        &self,
        failed_node_run_id: &WorkflowNodeRunId,
        failure: &NodeFailure,
        retry: &NodeRetryToSchedule,
        now: i64,
    ) -> Result<ScheduleNodeRetryResult, RepositoryError>;

    /// Starts a waiting attempt whose `due_at` has passed: removes `payload.retry_wait` and sets
    /// `started_at`.
    fn begin_node_retry(
        &self,
        node_run_id: &WorkflowNodeRunId,
        now: i64,
    ) -> Result<BeginNodeRetryResult, RepositoryError>;
}

/// Wakes waiting attempts when their backoff elapses.
///
/// `arm` must return immediately. At or after `due_at` the implementation calls
/// [`WorkflowRunEngine::wake_retry`](crate::WorkflowRunEngine::wake_retry) under the run's
/// serial gate. Wakes are idempotent, so a wake for a cancelled, failed, restarted, or already
/// started attempt is a harmless no-op, and nothing needs to be disarmed.
pub trait WorkflowRetryTimer: Send + Sync {
    fn arm(&self, run_id: &WorkflowRunId, node_run_id: &WorkflowNodeRunId, due_at: i64);
}

/// A timer that never wakes anything, for engines assembled without a backend timer.
pub struct NoRetryTimer;

impl WorkflowRetryTimer for NoRetryTimer {
    fn arm(&self, _run_id: &WorkflowRunId, _node_run_id: &WorkflowNodeRunId, _due_at: i64) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkflowGraph;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn policy(enabled: bool, max_retries: u32, initial_delay_seconds: u32) -> AgentRetryPolicy {
        AgentRetryPolicy {
            enabled,
            max_retries,
            initial_delay_seconds,
        }
    }

    /// Parses one agent node carrying `retry` through the same entry point publish and start use.
    fn parse_retry(retry: Value) -> Result<AgentRetryPolicy, String> {
        let graph = json!({"nodes": [{"id": "agent", "data": {"kind": "agent", "agentConfig": {
            "executor": {"agentCli": "c", "modelId": "m"}, "prompt": "p", "retry": retry
        }}}], "edges": []});
        WorkflowGraph::parse(&graph.to_string())
            .map(|graph| {
                graph
                    .node("agent")
                    .unwrap()
                    .agent_config
                    .clone()
                    .unwrap()
                    .retry
            })
            .map_err(|error| error.to_string())
    }

    /// A graph without `retry` gets two retries starting after ten seconds.
    #[test]
    fn missing_retry_defaults_to_two_retries_after_ten_seconds() {
        let graph = WorkflowGraph::parse(
            &json!({"nodes": [{"id": "agent", "data": {"kind": "agent", "agentConfig": {
                "executor": {"agentCli": "c", "modelId": "m"}, "prompt": "p"
            }}}], "edges": []})
            .to_string(),
        )
        .unwrap();
        assert_eq!(
            graph
                .node("agent")
                .unwrap()
                .agent_config
                .as_ref()
                .unwrap()
                .retry,
            policy(true, 2, 10)
        );
        assert_eq!(parse_retry(Value::Null), Ok(policy(true, 2, 10)));
    }

    /// Every in-range combination, including both bounds, is accepted verbatim.
    #[test]
    fn accepts_complete_in_range_policies() {
        for (retry, expected) in [
            (
                json!({"enabled": true, "maxRetries": 0, "initialDelaySeconds": 0}),
                policy(true, 0, 0),
            ),
            (
                json!({"enabled": false, "maxRetries": 5, "initialDelaySeconds": 300}),
                policy(false, 5, 300),
            ),
            (
                json!({"enabled": true, "maxRetries": 3, "initialDelaySeconds": 7, "extra": 1}),
                policy(true, 3, 7),
            ),
        ] {
            assert_eq!(parse_retry(retry), Ok(expected));
        }
    }

    /// Each malformed policy is rejected at graph parse with a message naming the node, the field,
    /// and the offending value.
    #[test]
    fn rejects_incomplete_or_out_of_range_policies_with_a_clear_message() {
        let prefix = "node agent has an invalid retry config: ";
        for (retry, reason) in [
            (json!(true), "retry must be an object, got true"),
            (json!([]), "retry must be an object, got []"),
            (
                json!({"maxRetries": 2, "initialDelaySeconds": 10}),
                "retry.enabled is required",
            ),
            (
                json!({"enabled": "yes", "maxRetries": 2, "initialDelaySeconds": 10}),
                "retry.enabled must be a boolean, got \"yes\"",
            ),
            (
                json!({"enabled": true, "initialDelaySeconds": 10}),
                "retry.maxRetries is required",
            ),
            (
                json!({"enabled": true, "maxRetries": 6, "initialDelaySeconds": 10}),
                "retry.maxRetries must be an integer from 0 to 5, got 6",
            ),
            (
                json!({"enabled": true, "maxRetries": -1, "initialDelaySeconds": 10}),
                "retry.maxRetries must be an integer from 0 to 5, got -1",
            ),
            (
                json!({"enabled": true, "maxRetries": 1.5, "initialDelaySeconds": 10}),
                "retry.maxRetries must be an integer from 0 to 5, got 1.5",
            ),
            (
                json!({"enabled": true, "maxRetries": "2", "initialDelaySeconds": 10}),
                "retry.maxRetries must be an integer from 0 to 5, got \"2\"",
            ),
            (
                json!({"enabled": true, "maxRetries": 2}),
                "retry.initialDelaySeconds is required",
            ),
            (
                json!({"enabled": true, "maxRetries": 2, "initialDelaySeconds": 301}),
                "retry.initialDelaySeconds must be an integer from 0 to 300, got 301",
            ),
            (
                json!({"enabled": true, "maxRetries": 2, "initialDelaySeconds": null}),
                "retry.initialDelaySeconds must be an integer from 0 to 300, got null",
            ),
        ] {
            assert_eq!(parse_retry(retry), Err(format!("{prefix}{reason}")));
        }
    }

    /// Every edge case of each field: `null` and `{}`, each field missing, unknown keys, and
    /// negative, fractional, integral-float, string, boolean, null, overflowing, boundary, and
    /// just-out-of-range values for both numbers, plus numbers where `enabled` wants a boolean.
    #[test]
    fn parses_every_edge_case_of_each_field() {
        let prefix = "node agent has an invalid retry config: ";
        let complete = |field: &str, value: Value| {
            let mut retry = json!({"enabled": true, "maxRetries": 2, "initialDelaySeconds": 10});
            retry[field] = value;
            retry
        };
        let mut cases: Vec<(Value, Result<AgentRetryPolicy, String>)> = vec![
            (Value::Null, Ok(policy(true, 2, 10))),
            (json!({}), Err("retry.enabled is required".to_string())),
            (
                json!({"enabled": true, "maxRetries": 2, "initialDelaySeconds": 10, "backoff": "x", "nested": {"a": 1}}),
                Ok(policy(true, 2, 10)),
            ),
            (
                json!({"maxRetries": 2, "initialDelaySeconds": 10}),
                Err("retry.enabled is required".to_string()),
            ),
            (
                json!({"enabled": true, "initialDelaySeconds": 10}),
                Err("retry.maxRetries is required".to_string()),
            ),
            (
                json!({"enabled": true, "maxRetries": 2}),
                Err("retry.initialDelaySeconds is required".to_string()),
            ),
            (
                complete("enabled", json!(1)),
                Err("retry.enabled must be a boolean, got 1".to_string()),
            ),
            (
                complete("enabled", json!(0)),
                Err("retry.enabled must be a boolean, got 0".to_string()),
            ),
            (
                complete("enabled", Value::Null),
                Err("retry.enabled must be a boolean, got null".to_string()),
            ),
            (complete("enabled", json!(false)), Ok(policy(false, 2, 10))),
        ];
        for (field, max, set) in [
            (
                "maxRetries",
                5_u32,
                (|n: u32| policy(true, n, 10)) as fn(u32) -> AgentRetryPolicy,
            ),
            ("initialDelaySeconds", 300, |n: u32| policy(true, 2, n)),
        ] {
            let range = format!("retry.{field} must be an integer from 0 to {max}, got");
            for (value, shown) in [
                (json!(-1), "-1"),
                (json!(1.5), "1.5"),
                (json!(1.0), "1.0"),
                (json!("2"), "\"2\""),
                (json!(true), "true"),
                (json!(false), "false"),
                (Value::Null, "null"),
                (json!(4_294_967_296_u64), "4294967296"),
                (json!(max + 1), &(max + 1).to_string()),
            ] {
                cases.push((complete(field, value), Err(format!("{range} {shown}"))));
            }
            for accepted in [0, 1, max] {
                cases.push((complete(field, json!(accepted)), Ok(set(accepted))));
            }
        }
        for (retry, expected) in cases {
            let label = retry.to_string();
            assert_eq!(
                parse_retry(retry),
                expected.map_err(|reason| format!("{prefix}{reason}")),
                "{label}"
            );
        }
    }

    /// The wait doubles per retry from the initial delay and never exceeds ten minutes.
    #[test]
    fn waits_double_per_retry_and_are_capped_at_ten_minutes() {
        let waits = |policy: AgentRetryPolicy| -> Vec<u64> {
            (1..=policy.max_retries)
                .map(|retry| policy.wait_seconds(retry))
                .collect()
        };
        assert_eq!(waits(policy(true, 2, 10)), vec![10, 20]);
        assert_eq!(waits(policy(true, 5, 300)), vec![300, 600, 600, 600, 600]);
        assert_eq!(waits(policy(true, 5, 50)), vec![50, 100, 200, 400, 600]);
        assert_eq!(waits(policy(true, 3, 0)), vec![0, 0, 0]);
        // Out-of-contract retry numbers still saturate at the cap instead of overflowing.
        assert_eq!(policy(true, 5, 300).wait_seconds(200), 600);
    }

    /// Only an enabled policy with budget left retries, and only for auto-retry kinds.
    #[test]
    fn next_retry_requires_an_enabled_policy_budget_and_a_retryable_kind() {
        let kind = NodeFailureKind::StructuredOutput;
        assert_eq!(policy(true, 2, 10).next_retry(kind, 0), Some(1));
        assert_eq!(policy(true, 2, 10).next_retry(kind, 1), Some(2));
        assert_eq!(policy(true, 2, 10).next_retry(kind, 2), None);
        assert_eq!(policy(true, 0, 10).next_retry(kind, 0), None);
        assert_eq!(policy(false, 2, 10).next_retry(kind, 0), None);
        assert_eq!(
            policy(true, 2, 10).next_retry(NodeFailureKind::PromptTemplate, 0),
            None
        );
    }

    /// The retry counter is read back from an attempt's payload; anything else is a first try.
    #[test]
    fn auto_retry_marker_round_trips_through_the_payload() {
        let payload = json!({"checkpoint": "abc", AUTO_RETRY_KEY: {"retry": 2, "max_retries": 3}});
        assert_eq!(
            NodeAutoRetry::from_payload(Some(&payload.to_string())),
            Some(NodeAutoRetry {
                retry: 2,
                max_retries: 3
            })
        );
        assert_eq!(
            NodeAutoRetry::from_payload(Some(r#"{"checkpoint":"abc"}"#)),
            None
        );
        assert_eq!(NodeAutoRetry::from_payload(Some("not json")), None);
        assert_eq!(NodeAutoRetry::from_payload(None), None);
    }
}
