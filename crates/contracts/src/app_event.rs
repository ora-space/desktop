use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ScheduledTaskEvent;

/// Describes an application-level invalidation or stream lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "app-event.ts")]
pub enum AppEvent {
    /// Confirms that the application stream is subscribed and may be consumed.
    Ready,
    /// Tells clients that the persisted session row should be queried again.
    SessionTitleUpdated { session_id: String },
    /// Tells clients that the cached plugin lifecycle snapshot should be queried again.
    PluginStatusChanged { plugin_id: String },
    /// Tells clients that commands discovered from this agent were discovered with other Skills.
    AgentModelsInvalidated { agent_ref: String },
    /// Tells clients that one plugin's Host MCP health may have changed.
    ///
    /// Like `PluginStatusChanged`, the event carries no state of its own: clients re-query the
    /// secret-free health surface for that plugin identity. A lost or reordered event only leaves
    /// a stale view until the next probe result or manual re-detect.
    McpHealthChanged { plugin_id: String },
    /// Tells clients that the persisted workflow run state should be queried again.
    ///
    /// The event carries no workflow state: the run, node, and variable rows remain the only
    /// source of truth, and a lost or reordered event only leaves a stale view until the next
    /// event or refresh re-queries persistence.
    WorkflowRunInvalidated { run_id: String },
    /// Announces one Scheduled Task lifecycle fact; see `ScheduledTaskEvent` for payloads.
    ///
    /// Like the other invalidations, the event is best-effort: a missed announcement only
    /// leaves the client's view stale until the next refresh re-queries persistence.
    ScheduledTask(ScheduledTaskEvent),
}

/// Opens the application event stream without filtering or ownership metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "app-event.ts")]
pub struct WatchAppEventsRequest {}

/// Exports the application event contract family for the generated frontend package.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    AppEvent::export(config)?;
    WatchAppEventsRequest::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AppEvent, WatchAppEventsRequest};
    use pretty_assertions::assert_eq;

    /// Verifies the stream lifecycle and invalidation variants retain their wire shape.
    #[test]
    fn serializes_app_event_wire_shape() {
        assert_eq!(
            serde_json::to_value(AppEvent::Ready).expect("ready serializes"),
            serde_json::json!({ "type": "ready" }),
        );
        assert_eq!(
            serde_json::to_value(AppEvent::PluginStatusChanged {
                plugin_id: "ora.example".to_string(),
            })
            .expect("plugin invalidation serializes"),
            serde_json::json!({
                "type": "plugin_status_changed",
                "plugin_id": "ora.example",
            }),
        );
        assert_eq!(
            serde_json::to_value(AppEvent::SessionTitleUpdated {
                session_id: "session-1".to_string(),
            })
            .expect("title invalidation serializes"),
            serde_json::json!({
                "type": "session_title_updated",
                "session_id": "session-1",
            }),
        );
        // A run invalidation carries only the run id — never node states, outputs, or variables.
        assert_eq!(
            serde_json::to_value(AppEvent::WorkflowRunInvalidated {
                run_id: "run-1".to_string(),
            })
            .expect("run invalidation serializes"),
            serde_json::json!({
                "type": "workflow_run_invalidated",
                "run_id": "run-1",
            }),
        );
        // An MCP health invalidation carries only the plugin identity, never a status body.
        assert_eq!(
            serde_json::to_value(AppEvent::McpHealthChanged {
                plugin_id: "official/tavily".to_string(),
            })
            .expect("mcp health invalidation serializes"),
            serde_json::json!({
                "type": "mcp_health_changed",
                "plugin_id": "official/tavily",
            }),
        );
        // Scheduled task announcements carry only navigation identifiers; the run rows stay
        // the single source of truth for status and history.
        assert_eq!(
            serde_json::to_value(AppEvent::ScheduledTask(
                crate::ScheduledTaskEvent::RunStarted {
                    task_id: "task-1".to_string(),
                    session_id: "session-1".to_string(),
                    worktree_task_id: "worktree-task-1".to_string(),
                    project_id: "project-1".to_string(),
                }
            ))
            .expect("scheduled run start serializes"),
            serde_json::json!({
                "type": "scheduled_task",
                "kind": "run_started",
                "taskId": "task-1",
                "sessionId": "session-1",
                "worktreeTaskId": "worktree-task-1",
                "projectId": "project-1",
            }),
        );
        assert_eq!(
            serde_json::to_value(AppEvent::ScheduledTask(
                crate::ScheduledTaskEvent::RunFinished {
                    task_id: "task-1".to_string(),
                    run_id: "run-1".to_string(),
                    status: crate::ScheduledRunStatus::Failed,
                    failure_reason: Some(crate::ScheduledRunFailureReason::Timeout),
                }
            ))
            .expect("scheduled run finish serializes"),
            serde_json::json!({
                "type": "scheduled_task",
                "kind": "run_finished",
                "taskId": "task-1",
                "runId": "run-1",
                "status": "failed",
                "failureReason": "timeout",
            }),
        );
        assert_eq!(
            serde_json::to_value(AppEvent::ScheduledTask(
                crate::ScheduledTaskEvent::OccurrenceMissed {
                    task_id: "task-1".to_string(),
                    scheduled_at: 1_000,
                    reason: crate::ScheduledMissReason::AppNotRunning,
                }
            ))
            .expect("missed occurrence serializes"),
            serde_json::json!({
                "type": "scheduled_task",
                "kind": "occurrence_missed",
                "taskId": "task-1",
                "scheduledAt": 1_000,
                "reason": "app_not_running",
            }),
        );
    }

    /// Verifies the application stream request carries no browser ownership state.
    #[test]
    fn serializes_empty_watch_request() {
        assert_eq!(
            serde_json::to_value(WatchAppEventsRequest {}).expect("watch request serializes"),
            serde_json::json!({}),
        );
    }
}
