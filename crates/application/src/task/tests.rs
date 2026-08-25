use super::mapper::map_task;
use ora_domain::{AuditFields, ProjectId, Task, TaskId, WorkspaceId};
use pretty_assertions::assert_eq;

/// Verifies the task projection exposes its workspace without any workflow association.
#[test]
fn task_projection_maps_direct_workspace_identity() {
    let task = Task::new(
        TaskId::new("task-1"),
        ProjectId::new("project-1"),
        WorkspaceId::new("workspace-1"),
        "Implement",
        AuditFields::new(1, 1, false),
    );
    let mapped = map_task(task);
    assert_eq!(mapped.workspace_id, "workspace-1");
    assert_eq!(mapped.project_id, "project-1");
}
