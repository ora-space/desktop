use ora_contracts::{
    ScheduledMissReason as ContractMissReason, ScheduledRunFailureReason as ContractFailureReason,
    ScheduledRunStatus as ContractRunStatus, ScheduledRunVerdict as ContractRunVerdict,
    ScheduledTask as ContractScheduledTask, ScheduledTaskApproval as ContractApproval,
    ScheduledTaskApprovalPolicy as ContractApprovalPolicy,
    ScheduledTaskExecutionLocation as ContractExecutionLocation,
    ScheduledTaskHistoryEntry as ContractHistoryEntry, ScheduledTaskSchedule as ContractSchedule,
    ScheduledTaskWeekday as ContractWeekday,
};
use ora_domain::{
    ScheduledMissReason, ScheduledRunFailureReason, ScheduledRunStatus, ScheduledRunVerdict,
    ScheduledTask, ScheduledTaskApproval, ScheduledTaskExecutionLocation,
    ScheduledTaskHistoryEntry, ScheduledTaskSchedule, ScheduledTaskWeekday,
};

use crate::ApplicationError;

/// Validates and converts the public schedule shape into the domain representation.
pub(crate) fn parse_schedule(
    schedule: ContractSchedule,
) -> Result<ScheduledTaskSchedule, ApplicationError> {
    match schedule {
        ContractSchedule::EveryHours { interval } => ScheduledTaskSchedule::every_hours(interval),
        ContractSchedule::Daily { hour, minute } => ScheduledTaskSchedule::daily(hour, minute),
        ContractSchedule::Weekly {
            weekdays,
            hour,
            minute,
        } => ScheduledTaskSchedule::weekly(
            weekdays.into_iter().map(map_weekday_to_domain).collect(),
            hour,
            minute,
        ),
    }
    .map_err(ApplicationError::from_scheduled_task_domain_error)
}

pub(crate) fn map_history_entry(entry: ScheduledTaskHistoryEntry) -> ContractHistoryEntry {
    match entry {
        ScheduledTaskHistoryEntry::Run(run) => ContractHistoryEntry::Run {
            id: run.id.to_string(),
            scheduled_at: run.occurrence.scheduled_at,
            revision: run.snapshot.revision,
            status: map_run_status(run.status),
            session_id: run.session_id.map(|id| id.to_string()),
            worktree_path: run.worktree_path,
            worktree_task_id: run.worktree_task_id.map(|id| id.to_string()),
            failure_reason: run.failure_reason.map(map_failure_reason),
            verdict: run.verdict.map(map_verdict),
            verdict_detail: run.verdict_detail,
            verdict_at: run.verdict_at,
            created_at: run.created_at,
            updated_at: run.updated_at,
        },
        ScheduledTaskHistoryEntry::Missed(missed) => ContractHistoryEntry::Missed {
            scheduled_at: missed.occurrence.scheduled_at,
            reason: match missed.reason {
                ScheduledMissReason::AppNotRunning => ContractMissReason::AppNotRunning,
                ScheduledMissReason::LateTrigger => ContractMissReason::LateTrigger,
            },
            observed_at: missed.observed_at,
        },
    }
}

fn map_failure_reason(reason: ScheduledRunFailureReason) -> ContractFailureReason {
    match reason {
        ScheduledRunFailureReason::TargetUnavailable => ContractFailureReason::TargetUnavailable,
        ScheduledRunFailureReason::ModelUnavailable => ContractFailureReason::ModelUnavailable,
        ScheduledRunFailureReason::WorktreeFailed => ContractFailureReason::WorktreeFailed,
        ScheduledRunFailureReason::ApprovalPolicyInvalid => {
            ContractFailureReason::ApprovalPolicyInvalid
        }
        ScheduledRunFailureReason::Timeout => ContractFailureReason::Timeout,
        ScheduledRunFailureReason::ExecutionError => ContractFailureReason::ExecutionError,
    }
}

fn map_run_status(status: ScheduledRunStatus) -> ContractRunStatus {
    match status {
        ScheduledRunStatus::Accepted => ContractRunStatus::Accepted,
        ScheduledRunStatus::Running => ContractRunStatus::Running,
        ScheduledRunStatus::Succeeded => ContractRunStatus::Succeeded,
        ScheduledRunStatus::Failed => ContractRunStatus::Failed,
        ScheduledRunStatus::Cancelled => ContractRunStatus::Cancelled,
        ScheduledRunStatus::SkippedBusy => ContractRunStatus::SkippedBusy,
        ScheduledRunStatus::SkippedCapacity => ContractRunStatus::SkippedCapacity,
    }
}

fn map_verdict(verdict: ScheduledRunVerdict) -> ContractRunVerdict {
    match verdict {
        ScheduledRunVerdict::Pass => ContractRunVerdict::Pass,
        ScheduledRunVerdict::Warn => ContractRunVerdict::Warn,
        ScheduledRunVerdict::Fail => ContractRunVerdict::Fail,
        ScheduledRunVerdict::Blocked => ContractRunVerdict::Blocked,
        ScheduledRunVerdict::Idle => ContractRunVerdict::Idle,
        ScheduledRunVerdict::RangeUnresolved => ContractRunVerdict::RangeUnresolved,
    }
}

pub(crate) fn approval_from_policy(
    policy: ContractApprovalPolicy,
    consented_at: i64,
) -> ScheduledTaskApproval {
    match policy {
        ContractApprovalPolicy::FullAutoInWorktree => {
            ScheduledTaskApproval::full_auto_in_worktree(consented_at)
        }
    }
}

/// Projects one domain task into the public contract form.
pub(crate) fn map_task(task: ScheduledTask) -> ContractScheduledTask {
    ContractScheduledTask {
        id: task.id.to_string(),
        name: task.name,
        enabled: task.enabled,
        schedule: map_schedule_to_contract(task.schedule),
        execution_location: match task.execution_location {
            ScheduledTaskExecutionLocation::Local => ContractExecutionLocation::Local,
            ScheduledTaskExecutionLocation::Cloud => ContractExecutionLocation::Cloud,
        },
        project_id: task.project_id.to_string(),
        agent_ref: task.agent_ref.to_string(),
        model: task.model,
        prompt: task.prompt,
        revision: task.revision,
        approval: task.approval.and_then(map_approval),
        worktree_base_branch: task.worktree_base_branch,
        created_at: task.audit_fields.created_at,
        updated_at: task.audit_fields.updated_at,
    }
}

fn map_approval(approval: ScheduledTaskApproval) -> Option<ContractApproval> {
    approval.is_supported().then_some(ContractApproval {
        policy: ContractApprovalPolicy::FullAutoInWorktree,
        consented_at: approval.consented_at,
    })
}

fn map_schedule_to_contract(schedule: ScheduledTaskSchedule) -> ContractSchedule {
    match schedule {
        ScheduledTaskSchedule::EveryHours { interval } => ContractSchedule::EveryHours { interval },
        ScheduledTaskSchedule::Daily { hour, minute } => ContractSchedule::Daily { hour, minute },
        ScheduledTaskSchedule::Weekly {
            weekdays,
            hour,
            minute,
        } => ContractSchedule::Weekly {
            weekdays: weekdays.into_iter().map(map_weekday_to_contract).collect(),
            hour,
            minute,
        },
    }
}

fn map_weekday_to_domain(weekday: ContractWeekday) -> ScheduledTaskWeekday {
    match weekday {
        ContractWeekday::Monday => ScheduledTaskWeekday::Monday,
        ContractWeekday::Tuesday => ScheduledTaskWeekday::Tuesday,
        ContractWeekday::Wednesday => ScheduledTaskWeekday::Wednesday,
        ContractWeekday::Thursday => ScheduledTaskWeekday::Thursday,
        ContractWeekday::Friday => ScheduledTaskWeekday::Friday,
        ContractWeekday::Saturday => ScheduledTaskWeekday::Saturday,
        ContractWeekday::Sunday => ScheduledTaskWeekday::Sunday,
    }
}

fn map_weekday_to_contract(weekday: ScheduledTaskWeekday) -> ContractWeekday {
    match weekday {
        ScheduledTaskWeekday::Monday => ContractWeekday::Monday,
        ScheduledTaskWeekday::Tuesday => ContractWeekday::Tuesday,
        ScheduledTaskWeekday::Wednesday => ContractWeekday::Wednesday,
        ScheduledTaskWeekday::Thursday => ContractWeekday::Thursday,
        ScheduledTaskWeekday::Friday => ContractWeekday::Friday,
        ScheduledTaskWeekday::Saturday => ContractWeekday::Saturday,
        ScheduledTaskWeekday::Sunday => ContractWeekday::Sunday,
    }
}
