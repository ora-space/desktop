//! Desktop workflow run operations.

use ora_contracts::*;

backend_command!(
    create_workflow_run,
    CreateWorkflowRunRequest,
    CreateWorkflowRunResponse,
    workflow_runs.create,
    "Creates one workflow run through the shared Backend."
);
backend_command!(
    get_workflow_run,
    GetWorkflowRunRequest,
    GetWorkflowRunResponse,
    workflow_runs.get,
    "Gets one workflow run through the shared Backend."
);
backend_command!(
    list_workflow_runs,
    ListWorkflowRunsRequest,
    ListWorkflowRunsResponse,
    workflow_runs.list,
    "Lists workflow runs for one project through the shared Backend."
);
backend_command!(
    list_workflow_runs_by_workflow,
    ListWorkflowRunsByWorkflowRequest,
    ListWorkflowRunsByWorkflowResponse,
    workflow_runs.list_by_workflow,
    "Lists workflow runs for one workflow through the shared Backend."
);
backend_command!(
    list_workflow_node_runs,
    ListWorkflowNodeRunsRequest,
    ListWorkflowNodeRunsResponse,
    workflow_runs.list_node_runs,
    "Lists the node-run history of one workflow run through the shared Backend."
);
backend_command!(
    delete_workflow_run,
    DeleteWorkflowRunRequest,
    DeleteWorkflowRunResponse,
    workflow_runs.delete,
    "Deletes one workflow run through the shared Backend."
);
backend_command!(
    rename_workflow_run,
    RenameWorkflowRunRequest,
    RenameWorkflowRunResponse,
    workflow_runs.rename,
    "Renames one workflow run through the shared Backend."
);
backend_command!(
    start_workflow_run,
    StartWorkflowRunRequest,
    StartWorkflowRunResponse,
    workflow_runs.start,
    "Starts one workflow run through the shared Backend."
);
async_backend_command!(
    cancel_workflow_run,
    CancelWorkflowRunRequest,
    CancelWorkflowRunResponse,
    workflow_runs.cancel,
    "Cancels one workflow run through its owned interface."
);
backend_command!(
    restart_workflow_run,
    RestartWorkflowRunRequest,
    RestartWorkflowRunResponse,
    workflow_runs.restart,
    "Restarts one workflow run through the shared Backend."
);
backend_command!(
    update_workflow_run_input,
    UpdateWorkflowRunInputRequest,
    UpdateWorkflowRunInputResponse,
    workflow_runs.update_input,
    "Updates the kickoff input of one workflow run through the shared Backend."
);
async_backend_command!(
    complete_workflow_node,
    CompleteWorkflowNodeRequest,
    CompleteWorkflowNodeResponse,
    workflow_runs.complete_node,
    "Completes one awaiting interactive workflow node through its owned interface."
);
