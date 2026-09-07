//! Desktop project operations.

use ora_contracts::*;

backend_command!(
    create_project,
    CreateProjectRequest,
    CreateProjectResponse,
    projects.create,
    "Creates one project through the shared Backend."
);
backend_command!(
    get_project,
    GetProjectRequest,
    GetProjectResponse,
    projects.get,
    "Gets one project through the shared Backend."
);
backend_command!(
    list_projects,
    ListProjectsRequest,
    ListProjectsResponse,
    projects.list,
    "Lists projects through the shared Backend."
);
backend_command!(
    list_project_branches,
    ListProjectBranchesRequest,
    ListProjectBranchesResponse,
    projects.list_branches,
    "Lists local branches for one project through the shared Backend."
);
backend_command!(
    update_project,
    UpdateProjectRequest,
    UpdateProjectResponse,
    projects.update,
    "Updates one project through the shared Backend."
);
async_backend_command!(
    delete_project,
    DeleteProjectRequest,
    DeleteProjectResponse,
    projects.delete,
    "Commits the aggregate cascade and schedules its durable Git cleanup."
);
