use ora_contracts::{Task as ContractTask, TaskType as ContractTaskType, TaskWorkspaceMode};
use ora_domain::{Task as DomainTask, TaskType as DomainTaskType};

/// Maps a domain task into the app-facing contract shape.
pub(crate) fn map_task(task: DomainTask) -> ContractTask {
    let workspace_mode = match task.worktree_id {
        Some(_) => TaskWorkspaceMode::Worktree,
        None => TaskWorkspaceMode::ProjectRoot,
    };

    ContractTask {
        id: task.id.to_string(),
        project_id: task.project_id.to_string(),
        title: task.title,
        workspace_mode,
        task_type: map_task_type(task.task_type),
        workflow_run_id: task.workflow_run_id.map(|id| id.to_string()),
    }
}

/// Translates the internal task type into the transport-facing enum.
fn map_task_type(task_type: DomainTaskType) -> ContractTaskType {
    match task_type {
        DomainTaskType::Default => ContractTaskType::Default,
        DomainTaskType::Workflow => ContractTaskType::Workflow,
    }
}

#[cfg(test)]
mod tests {
    use super::map_task;
    use ora_contracts::TaskType as ContractTaskType;
    use ora_domain::{AuditFields, ProjectId, Task, TaskId, WorkflowRunId, WorktreeId};
    use pretty_assertions::assert_eq;

    /// Verifies a workflow-run task maps to the run task type with its run reference intact.
    #[test]
    fn maps_run_task_to_contract_type() {
        let mapped = map_task(Task::workflow_run(
            TaskId::new("task-1"),
            ProjectId::new("project-1"),
            "Workflow run",
            WorkflowRunId::new("run-1"),
            WorktreeId::new("worktree-1"),
            AuditFields::new(10, 10, /*is_deleted*/ false),
        ));

        assert_eq!(mapped.task_type, ContractTaskType::Workflow);
        assert_eq!(mapped.workflow_run_id, Some("run-1".to_string()));
    }
}
