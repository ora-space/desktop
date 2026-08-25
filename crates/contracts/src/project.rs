use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes the public project payload shared across adapter responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct Project {
    pub id: String,
    pub name: String,
}

/// Carries the app-facing payload for project creation requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct CreateProjectRequest {
    pub name: String,
    pub main_workspace_path: String,
}

/// Returns the created project after a successful create request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct CreateProjectResponse {
    pub project: Project,
}

/// Identifies which project to fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct GetProjectRequest {
    pub project_id: String,
}

/// Returns one project payload after a successful fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct GetProjectResponse {
    pub project: Project,
}

/// Requests the full visible project list.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct ListProjectsRequest {}

/// Returns the visible project list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct ListProjectsResponse {
    pub projects: Vec<Project>,
}

/// Identifies which project's local Git branches should be listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct ListProjectBranchesRequest {
    pub project_id: String,
}

/// Returns local branches that can seed a new worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct ListProjectBranchesResponse {
    pub branches: Vec<ProjectBranch>,
}

/// Separates the logical branch name, resolvable ref, and label shown to users.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct ProjectBranch {
    pub name: String,
    pub ref_name: String,
    pub display_name: String,
}

/// Carries the mutable project name without changing the main workspace location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct UpdateProjectRequest {
    pub project_id: String,
    pub name: String,
}

/// Returns the updated project after a successful update request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct UpdateProjectResponse {
    pub project: Project,
}

/// Identifies which project to delete.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct DeleteProjectRequest {
    pub project_id: String,
}

/// Returns the deleted project identifier after a successful delete request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "project.ts")]
pub struct DeleteProjectResponse {
    pub project_id: String,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    Project::export(config)?;
    CreateProjectRequest::export(config)?;
    CreateProjectResponse::export(config)?;
    GetProjectRequest::export(config)?;
    GetProjectResponse::export(config)?;
    ListProjectsRequest::export(config)?;
    ListProjectsResponse::export(config)?;
    ListProjectBranchesRequest::export(config)?;
    ListProjectBranchesResponse::export(config)?;
    ProjectBranch::export(config)?;
    UpdateProjectRequest::export(config)?;
    UpdateProjectResponse::export(config)?;
    DeleteProjectRequest::export(config)?;
    DeleteProjectResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest, DeleteProjectResponse,
        GetProjectRequest, GetProjectResponse, ListProjectBranchesRequest,
        ListProjectBranchesResponse, ListProjectsRequest, ListProjectsResponse, Project,
        ProjectBranch, UpdateProjectRequest, UpdateProjectResponse,
    };
    use pretty_assertions::assert_eq;
    use serde::Serialize;
    use serde_json::{Value, json};

    /// Verifies the first project slice serializes to frontend-friendly JSON payloads.
    #[test]
    fn serializes_project_contracts() {
        let project = Project {
            id: "project-1".to_string(),
            name: "Ora".to_string(),
        };
        let create_request = CreateProjectRequest {
            name: "Ora".to_string(),
            main_workspace_path: "/workspace/ora".to_string(),
        };
        let get_request = GetProjectRequest {
            project_id: "project-1".to_string(),
        };
        let list_request = ListProjectsRequest {};
        let list_branches_request = ListProjectBranchesRequest {
            project_id: "project-1".to_string(),
        };
        let update_request = UpdateProjectRequest {
            project_id: "project-1".to_string(),
            name: "Ora Updated".to_string(),
        };
        let delete_request = DeleteProjectRequest {
            project_id: "project-1".to_string(),
        };

        assert_serialized_json(
            &project,
            json!({
                "id": "project-1",
                "name": "Ora",
            }),
        );
        assert_serialized_json(
            &create_request,
            json!({
                "name": "Ora",
                "mainWorkspacePath": "/workspace/ora",
            }),
        );
        assert_serialized_json(
            &CreateProjectResponse {
                project: project.clone(),
            },
            json!({
                "project": {
                    "id": "project-1",
                    "name": "Ora",
                },
            }),
        );
        assert_serialized_json(&get_request, json!({ "projectId": "project-1" }));
        assert_serialized_json(
            &GetProjectResponse {
                project: project.clone(),
            },
            json!({
                "project": {
                    "id": "project-1",
                    "name": "Ora",
                },
            }),
        );
        assert_serialized_json(&list_request, json!({}));
        assert_serialized_json(&list_branches_request, json!({ "projectId": "project-1" }));
        assert_serialized_json(
            &ListProjectBranchesResponse {
                branches: vec![ProjectBranch {
                    name: "main".to_string(),
                    ref_name: "origin/main".to_string(),
                    display_name: "main".to_string(),
                }],
            },
            json!({
                "branches": [{
                    "name": "main",
                    "refName": "origin/main",
                    "displayName": "main",
                }],
            }),
        );
        assert_serialized_json(
            &ListProjectsResponse {
                projects: vec![project.clone()],
            },
            json!({
                "projects": [
                    {
                        "id": "project-1",
                        "name": "Ora",
                    },
                ],
            }),
        );
        assert_serialized_json(
            &update_request,
            json!({
                "projectId": "project-1",
                "name": "Ora Updated",
            }),
        );
        assert_serialized_json(
            &UpdateProjectResponse { project },
            json!({
                "project": {
                    "id": "project-1",
                    "name": "Ora",
                },
            }),
        );
        assert_serialized_json(&delete_request, json!({ "projectId": "project-1" }));
        assert_serialized_json(
            &DeleteProjectResponse {
                project_id: "project-1".to_string(),
            },
            json!({ "projectId": "project-1" }),
        );
    }

    /// Confirms the shared project view remains the single reusable payload across responses.
    #[test]
    fn preserves_shared_project_shape_across_responses() {
        let project = Project {
            id: "project-1".to_string(),
            name: "Ora".to_string(),
        };

        assert_eq!(
            CreateProjectResponse {
                project: project.clone(),
            },
            CreateProjectResponse {
                project: project.clone(),
            }
        );
        assert_eq!(
            GetProjectResponse {
                project: project.clone(),
            },
            GetProjectResponse {
                project: project.clone(),
            }
        );
        assert_eq!(
            ListProjectsResponse {
                projects: vec![project.clone()],
            },
            ListProjectsResponse {
                projects: vec![project.clone()],
            }
        );
        assert_eq!(
            UpdateProjectResponse {
                project: project.clone()
            },
            UpdateProjectResponse { project }
        );
    }

    /// Serializes a contract value and compares it against the expected JSON structure.
    fn assert_serialized_json<T>(value: &T, expected: Value)
    where
        T: Serialize,
    {
        let actual = match serde_json::to_value(value) {
            Ok(actual) => actual,
            Err(error) => panic!("failed to serialize contract value: {error}"),
        };

        assert_eq!(actual, expected);
    }
}
