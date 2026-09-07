//! Desktop task operations.

use ora_contracts::*;

backend_command!(
    create_task,
    CreateTaskRequest,
    CreateTaskResponse,
    tasks.create,
    "Creates one task through the shared Backend."
);
backend_command!(
    get_task,
    GetTaskRequest,
    GetTaskResponse,
    tasks.get,
    "Gets one task through the shared Backend."
);
backend_command!(
    list_tasks,
    ListTasksRequest,
    ListTasksResponse,
    tasks.list,
    "Lists tasks through the shared Backend."
);
backend_command!(
    update_task,
    UpdateTaskRequest,
    UpdateTaskResponse,
    tasks.update,
    "Updates one task through the shared Backend."
);
async_backend_command!(
    delete_task,
    DeleteTaskRequest,
    DeleteTaskResponse,
    tasks.delete,
    "Commits the aggregate cascade and schedules its durable Git cleanup."
);

backend_command!(
    get_task_workspace,
    GetTaskWorkspaceRequest,
    GetTaskWorkspaceResponse,
    tasks.workspace,
    "Returns the authoritative task root and optional linked-worktree branch."
);
