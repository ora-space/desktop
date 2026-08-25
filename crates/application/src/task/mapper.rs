use ora_contracts::Task as ContractTask;
use ora_domain::Task as DomainTask;

/// Maps a domain task into the app-facing contract shape.
pub(crate) fn map_task(task: DomainTask) -> ContractTask {
    ContractTask {
        id: task.id.to_string(),
        project_id: task.project_id.to_string(),
        workspace_id: task.workspace_id.to_string(),
        title: task.title,
    }
}

#[cfg(test)]
mod tests {
    use super::map_task;
    use ora_domain::{AuditFields, ProjectId, Task, TaskId, WorkspaceId};
    use pretty_assertions::assert_eq;

    /// Verifies a worktree label preserves its direct workspace identity.
    #[test]
    fn maps_worktree_task_to_workspace() {
        let mapped = map_task(Task::new(
            TaskId::new("task-1"),
            ProjectId::new("project-1"),
            WorkspaceId::new("workspace-1"),
            "Workflow run",
            AuditFields::new(10, 10, /*is_deleted*/ false),
        ));

        assert_eq!(mapped.workspace_id, "workspace-1");
    }
}
