use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::AgentRef;

/// Selects one local weekday for a weekly scheduled task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "scheduled-task.ts")]
pub enum ScheduledTaskWeekday {
    Monday,
    Tuesday,
    Wednesday,
    Thursday,
    Friday,
    Saturday,
    Sunday,
}

/// Describes one of the bounded first-release schedule shapes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "scheduled-task.ts")]
pub enum ScheduledTaskSchedule {
    EveryHours {
        interval: u16,
    },
    Daily {
        hour: u8,
        minute: u8,
    },
    Weekly {
        weekdays: BTreeSet<ScheduledTaskWeekday>,
        hour: u8,
        minute: u8,
    },
}

/// Identifies which runtime owns every future occurrence of a task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "scheduled-task.ts")]
pub enum ScheduledTaskExecutionLocation {
    Local,
    Cloud,
}

/// Names an unattended execution policy the user has explicitly accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "scheduled-task.ts")]
pub enum ScheduledTaskApprovalPolicy {
    FullAutoInWorktree,
}

/// States how an edit changes the persisted unattended-execution consent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "scheduled-task.ts")]
pub enum ScheduledTaskApprovalChange {
    /// Leaves the current consent fact unchanged unless another edit invalidates it.
    Keep,
    /// Records a fresh, explicit consent fact for the selected policy.
    Grant { policy: ScheduledTaskApprovalPolicy },
    /// Removes consent and requires the task to be disabled.
    Revoke,
}

/// Returns the persisted consent fact without exposing runtime implementation details.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct ScheduledTaskApproval {
    pub policy: ScheduledTaskApprovalPolicy,
    pub consented_at: i64,
}

/// Public scheduled task definition shown in the Desktop list and editor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct ScheduledTask {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub schedule: ScheduledTaskSchedule,
    pub execution_location: ScheduledTaskExecutionLocation,
    pub project_id: String,
    pub agent_ref: AgentRef,
    pub model: Option<String>,
    pub prompt: String,
    pub revision: u64,
    pub approval: Option<ScheduledTaskApproval>,
    /// Branch frozen as the base of every run's worktree. `None` only for definitions an older
    /// build stored before freezing; the backend then resolves it from the project on the next
    /// save, which needs a Git repository, so the editor must know.
    pub worktree_base_branch: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Carries the editable fields used to create one local scheduled task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct CreateScheduledTaskRequest {
    pub name: String,
    pub enabled: bool,
    pub schedule: ScheduledTaskSchedule,
    pub execution_location: ScheduledTaskExecutionLocation,
    pub project_id: String,
    pub agent_ref: AgentRef,
    pub model: Option<String>,
    pub prompt: String,
    pub approval_policy: Option<ScheduledTaskApprovalPolicy>,
}

/// Returns the newly persisted scheduled task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct CreateScheduledTaskResponse {
    pub task: ScheduledTask,
}

/// Identifies one scheduled task definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct GetScheduledTaskRequest {
    pub task_id: String,
}

/// Returns one scheduled task definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct GetScheduledTaskResponse {
    pub task: ScheduledTask,
}

/// Requests every visible scheduled task definition.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct ListScheduledTasksRequest {}

/// Returns every visible scheduled task definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct ListScheduledTasksResponse {
    pub tasks: Vec<ScheduledTask>,
}

/// Mirrors the persisted Scheduled Run lifecycle states used by task history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "scheduled-task.ts")]
pub enum ScheduledRunStatus {
    Accepted,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    SkippedBusy,
    SkippedCapacity,
}

/// Explains why an accepted Scheduled Run ended in the failed state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "scheduled-task.ts")]
pub enum ScheduledRunFailureReason {
    TargetUnavailable,
    ModelUnavailable,
    WorktreeFailed,
    ApprovalPolicyInvalid,
    Timeout,
    ExecutionError,
}

/// Independent, machine-readable business conclusion a run's agent reported.
///
/// It is nullable: old histories and runs whose agent never wrote a verdict read as
/// "not reported". It is independent of `ScheduledRunStatus` and
/// `ScheduledRunFailureReason`, so a run can be `succeeded` while its verdict is `fail`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "scheduled-task.ts")]
pub enum ScheduledRunVerdict {
    Pass,
    Warn,
    Fail,
    Blocked,
    Idle,
    RangeUnresolved,
}

/// Explains why a planned occurrence did not create a Scheduled Run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "scheduled-task.ts")]
pub enum ScheduledMissReason {
    AppNotRunning,
    LateTrigger,
}

/// Returns one durable run or missed fact without exposing the frozen prompt snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "scheduled-task.ts")]
pub enum ScheduledTaskHistoryEntry {
    Run {
        id: String,
        scheduled_at: i64,
        revision: u64,
        status: ScheduledRunStatus,
        session_id: Option<String>,
        worktree_path: Option<String>,
        /// The task-owned worktree this run executed in; present only after the run started.
        worktree_task_id: Option<String>,
        failure_reason: Option<ScheduledRunFailureReason>,
        verdict: Option<ScheduledRunVerdict>,
        verdict_detail: Option<String>,
        verdict_at: Option<i64>,
        created_at: i64,
        updated_at: i64,
    },
    Missed {
        scheduled_at: i64,
        reason: ScheduledMissReason,
        observed_at: i64,
    },
}

/// Mirrors the closed artifact type whitelist a run may declare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "scheduled-task.ts")]
pub enum ScheduledArtifactKind {
    Markdown,
    Json,
    Text,
    Log,
    Xml,
    Html,
}

/// Explains why a declared candidate was refused by artifact collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "scheduled-task.ts")]
pub enum ScheduledArtifactRejectionReason {
    UnsupportedType,
    PathUnsafe,
    Symlink,
    HardLink,
    TooLarge,
    TooMany,
    TotalTooLarge,
    ReadFailed,
    CopyFailed,
}

/// One collected artifact registered in a run's manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct ScheduledRunArtifactInfo {
    /// Forward-slash path relative to `.ora-scheduled-artifacts/`.
    pub relative_path: String,
    pub kind: ScheduledArtifactKind,
    pub size_bytes: u64,
    pub registered_at: i64,
}

/// One candidate artifact collection refused, kept for history visibility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct ScheduledRunArtifactRejectionInfo {
    pub name: String,
    pub reason: ScheduledArtifactRejectionReason,
}

/// Identifies one run whose artifact manifest should be returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct ListScheduledRunArtifactsRequest {
    pub run_id: String,
}

/// Returns one run's collected artifacts and rejected candidates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct ListScheduledRunArtifactsResponse {
    pub artifacts: Vec<ScheduledRunArtifactInfo>,
    pub rejections: Vec<ScheduledRunArtifactRejectionInfo>,
}

/// Identifies one registered artifact for an in-app preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct ReadScheduledRunArtifactRequest {
    pub run_id: String,
    pub relative_path: String,
}

/// Returns the text preview of one artifact, truncated when it exceeds the preview bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct ReadScheduledRunArtifactResponse {
    pub content: String,
    pub truncated: bool,
}

/// Identifies the local task whose persisted history should be returned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct ListScheduledTaskHistoryRequest {
    pub task_id: String,
}

/// Returns newest-first persisted history for one local task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct ListScheduledTaskHistoryResponse {
    pub entries: Vec<ScheduledTaskHistoryEntry>,
}

/// Replaces editable fields while preserving execution ownership and consent rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct UpdateScheduledTaskRequest {
    pub task_id: String,
    /// Snapshot version observed by the caller before this replacement was prepared.
    pub expected_updated_at: i64,
    pub name: String,
    pub enabled: bool,
    pub schedule: ScheduledTaskSchedule,
    pub project_id: String,
    pub agent_ref: AgentRef,
    pub model: Option<String>,
    pub prompt: String,
    pub approval_change: ScheduledTaskApprovalChange,
}

/// Announces one Scheduled Task lifecycle fact on the application event stream.
///
/// The event is an interactive aid, not a run fact: clients may miss it without any run row
/// changing, and the identifiers only ask the client to re-query persisted history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "scheduled-task.ts")]
pub enum ScheduledTaskEvent {
    /// One admitted Scheduled Run whose session and worktree are persisted.
    RunStarted {
        task_id: String,
        session_id: String,
        worktree_task_id: String,
        project_id: String,
    },
    /// One Scheduled Run that reached a terminal state.
    RunFinished {
        task_id: String,
        /// The finished Scheduled Run, so clients can claim exactly this fact instead of
        /// guessing which history entry a failure reason belongs to.
        run_id: String,
        status: ScheduledRunStatus,
        failure_reason: Option<ScheduledRunFailureReason>,
    },
    /// One planned occurrence that was persisted as missed.
    OccurrenceMissed {
        task_id: String,
        scheduled_at: i64,
        reason: ScheduledMissReason,
    },
}

/// Returns the authoritative updated task snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct UpdateScheduledTaskResponse {
    pub task: ScheduledTask,
}

/// Identifies a task to soft-delete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct DeleteScheduledTaskRequest {
    pub task_id: String,
}

/// Confirms which task definition was soft-deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct DeleteScheduledTaskResponse {
    pub task_id: String,
}

/// Revokes unattended consent and disables the task without rewriting history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct RevokeScheduledTaskApprovalRequest {
    pub task_id: String,
}

/// Returns the disabled task after consent is revoked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "scheduled-task.ts")]
pub struct RevokeScheduledTaskApprovalResponse {
    pub task: ScheduledTask,
}

/// Exports every Scheduled Task TypeScript binding into the generated contracts package.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    ScheduledTaskWeekday::export(config)?;
    ScheduledTaskSchedule::export(config)?;
    ScheduledTaskExecutionLocation::export(config)?;
    ScheduledTaskApprovalPolicy::export(config)?;
    ScheduledTaskApprovalChange::export(config)?;
    ScheduledTaskApproval::export(config)?;
    ScheduledTask::export(config)?;
    CreateScheduledTaskRequest::export(config)?;
    CreateScheduledTaskResponse::export(config)?;
    GetScheduledTaskRequest::export(config)?;
    GetScheduledTaskResponse::export(config)?;
    ListScheduledTasksRequest::export(config)?;
    ListScheduledTasksResponse::export(config)?;
    ScheduledRunStatus::export(config)?;
    ScheduledRunFailureReason::export(config)?;
    ScheduledRunVerdict::export(config)?;
    ScheduledMissReason::export(config)?;
    ScheduledArtifactKind::export(config)?;
    ScheduledArtifactRejectionReason::export(config)?;
    ScheduledRunArtifactInfo::export(config)?;
    ScheduledRunArtifactRejectionInfo::export(config)?;
    ListScheduledRunArtifactsRequest::export(config)?;
    ListScheduledRunArtifactsResponse::export(config)?;
    ReadScheduledRunArtifactRequest::export(config)?;
    ReadScheduledRunArtifactResponse::export(config)?;
    ScheduledTaskHistoryEntry::export(config)?;
    ScheduledTaskEvent::export(config)?;
    ListScheduledTaskHistoryRequest::export(config)?;
    ListScheduledTaskHistoryResponse::export(config)?;
    UpdateScheduledTaskRequest::export(config)?;
    UpdateScheduledTaskResponse::export(config)?;
    DeleteScheduledTaskRequest::export(config)?;
    DeleteScheduledTaskResponse::export(config)?;
    RevokeScheduledTaskApprovalRequest::export(config)?;
    RevokeScheduledTaskApprovalResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::{
        ScheduledMissReason, ScheduledTaskHistoryEntry, ScheduledTaskSchedule, ScheduledTaskWeekday,
    };

    #[test]
    fn weekly_schedule_uses_the_public_tagged_shape() {
        let schedule = ScheduledTaskSchedule::Weekly {
            weekdays: BTreeSet::from([ScheduledTaskWeekday::Monday, ScheduledTaskWeekday::Friday]),
            hour: 9,
            minute: 30,
        };

        assert_eq!(
            serde_json::to_value(schedule).expect("serialize weekly schedule"),
            json!({
                "kind": "weekly",
                "weekdays": ["monday", "friday"],
                "hour": 9,
                "minute": 30
            })
        );
    }

    #[test]
    fn missed_history_uses_the_public_tagged_shape() {
        let entry = ScheduledTaskHistoryEntry::Missed {
            scheduled_at: 100,
            reason: ScheduledMissReason::LateTrigger,
            observed_at: 700_001,
        };

        assert_eq!(
            serde_json::to_value(entry).expect("serialize missed history"),
            json!({
                "kind": "missed",
                "scheduledAt": 100,
                "reason": "late_trigger",
                "observedAt": 700_001
            })
        );
    }
}
