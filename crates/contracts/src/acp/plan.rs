use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, serde_as, skip_serializing_none};
use ts_rs::TS;

/// An execution plan for accomplishing complex tasks.
///
/// Plans consist of multiple entries representing individual tasks or goals.
/// Agents report plans to clients to provide visibility into their execution strategy.
/// Plans can evolve during execution as the agent discovers new requirements or completes tasks.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/plan.ts")]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// The list of tasks to be accomplished.
    ///
    /// When updating a plan, the agent must send a complete list of all entries
    /// with their current status. The client replaces the entire plan with each update.
    #[serde_as(deserialize_as = "DefaultOnError")]
    pub entries: Vec<PlanEntry>,
}

impl Plan {
    /// Builds [`Plan`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(entries: Vec<PlanEntry>) -> Self {
        Self { entries }
    }
}

/// A single entry in the execution plan.
///
/// Represents a task or goal that the assistant intends to accomplish
/// as part of fulfilling the user's request.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export_to = "acp/plan.ts")]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    /// Human-readable description of what this task aims to accomplish.
    pub content: String,
    /// The relative importance of this task.
    /// Used to indicate which tasks are most critical to the overall goal.
    pub priority: PlanEntryPriority,
    /// Current execution status of this task.
    pub status: PlanEntryStatus,
}

impl PlanEntry {
    /// Builds [`PlanEntry`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(
        content: impl Into<String>,
        priority: PlanEntryPriority,
        status: PlanEntryStatus,
    ) -> Self {
        Self {
            content: content.into(),
            priority,
            status,
        }
    }
}

/// Priority levels for plan entries.
///
/// Used to indicate the relative importance or urgency of different
/// tasks in the execution plan.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, TS)]
#[ts(export_to = "acp/plan.ts")]
#[serde(rename_all = "snake_case")]
pub enum PlanEntryPriority {
    /// High priority task - critical to the overall goal.
    High,
    /// Medium priority task - important but not critical.
    Medium,
    /// Low priority task - nice to have but not essential.
    Low,
}

/// Status of a plan entry in the execution flow.
///
/// Tracks the lifecycle of each task from planning through completion.
#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq, TS)]
#[ts(export_to = "acp/plan.ts")]
#[serde(rename_all = "snake_case")]
pub enum PlanEntryStatus {
    /// The task has not started yet.
    Pending,
    /// The task is currently being worked on.
    InProgress,
    /// The task has been successfully completed.
    Completed,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    Plan::export(config)?;
    PlanEntry::export(config)?;
    PlanEntryPriority::export(config)?;
    PlanEntryStatus::export(config)?;
    Ok(())
}
