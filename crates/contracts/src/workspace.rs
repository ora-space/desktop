use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Identifies the canonical project checkout or an isolated execution environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace.ts")]
pub enum WorkspaceKind {
    Main,
    Isolated,
}

/// Reports the lifecycle admission state visible to workspace consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace.ts")]
pub enum WorkspaceLifecycle {
    Provisioning,
    Active,
    Unavailable,
    Retiring,
    Deleted,
}

/// Reports whether Git operations can act on a workspace directory that exists.
///
/// Projects may be plain folders. Ora asks Git about the directory each time it is listed, so
/// clients can hide Git-only features instead of surfacing their failures. A folder nested inside
/// another repository's checkout counts as `Repository`, because Git commands run from it act on
/// that enclosing checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace.ts")]
pub enum WorkspaceGit {
    /// Git resolves the directory to a checkout or linked worktree.
    Repository,
    /// Git explicitly reports that the directory is not inside any repository.
    NotRepository,
    /// Git could not answer (for example it is not installed, distrusts the owner, or the
    /// repository is damaged), so clients keep their usual behavior.
    Unknown,
}

/// Reports whether the directory behind a workspace exists at the moment it was listed.
///
/// Lifecycle admission does not cover this: a user can delete or move a project folder at any
/// time without telling Ora, and every session start, model discovery, and file operation then
/// fails. Probing on list lets clients explain that state instead of showing empty results.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export_to = "workspace.ts")]
pub enum WorkspaceDirectory {
    /// The local checkout exists as a directory; `git` says whether Git operations apply to it.
    Present { git: WorkspaceGit },
    /// The recorded local checkout is not a directory; `path` is the resolved location.
    Missing { path: String },
    /// The workspace is not a local checkout, so Desktop cannot probe it.
    Remote,
}

/// Describes the stable workspace identity used by Sessions and WorkflowRuns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace.ts")]
pub struct Workspace {
    pub id: String,
    pub project_id: String,
    pub kind: WorkspaceKind,
    pub lifecycle: WorkspaceLifecycle,
    pub directory: WorkspaceDirectory,
}

/// Requests every visible workspace so clients can resolve direct chats to the main workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace.ts")]
pub struct ListWorkspacesRequest {}

/// Returns the visible workspace identities in stable creation order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace.ts")]
pub struct ListWorkspacesResponse {
    pub workspaces: Vec<Workspace>,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    WorkspaceKind::export(config)?;
    WorkspaceLifecycle::export(config)?;
    WorkspaceGit::export(config)?;
    WorkspaceDirectory::export(config)?;
    Workspace::export(config)?;
    ListWorkspacesRequest::export(config)?;
    ListWorkspacesResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ListWorkspacesRequest, ListWorkspacesResponse, Workspace, WorkspaceDirectory, WorkspaceGit,
        WorkspaceKind, WorkspaceLifecycle,
    };
    use pretty_assertions::assert_eq;
    use serde::Serialize;
    use serde_json::{Value, json};

    /// Verifies workspace identity and lifecycle values use the frontend wire shape.
    #[test]
    fn serializes_workspace_contracts() {
        let workspace = Workspace {
            id: "workspace-1".to_string(),
            project_id: "project-1".to_string(),
            kind: WorkspaceKind::Main,
            lifecycle: WorkspaceLifecycle::Active,
            directory: WorkspaceDirectory::Missing {
                path: "/gone/ora".to_string(),
            },
        };

        assert_serialized_json(
            &workspace,
            json!({
                "id": "workspace-1",
                "projectId": "project-1",
                "kind": "main",
                "lifecycle": "active",
                "directory": { "kind": "missing", "path": "/gone/ora" },
            }),
        );
        assert_serialized_json(&ListWorkspacesRequest {}, json!({}));
        assert_serialized_json(
            &ListWorkspacesResponse {
                workspaces: vec![workspace],
            },
            json!({
                "workspaces": [{
                    "id": "workspace-1",
                    "projectId": "project-1",
                    "kind": "main",
                    "lifecycle": "active",
                    "directory": { "kind": "missing", "path": "/gone/ora" },
                }],
            }),
        );
    }

    /// Verifies a present directory carries its Git capability in the frontend wire shape.
    #[test]
    fn serializes_present_directory_git_capability() {
        assert_serialized_json(
            &WorkspaceDirectory::Present {
                git: WorkspaceGit::NotRepository,
            },
            json!({ "kind": "present", "git": "notRepository" }),
        );
        assert_serialized_json(
            &WorkspaceDirectory::Present {
                git: WorkspaceGit::Repository,
            },
            json!({ "kind": "present", "git": "repository" }),
        );
        assert_serialized_json(
            &WorkspaceDirectory::Present {
                git: WorkspaceGit::Unknown,
            },
            json!({ "kind": "present", "git": "unknown" }),
        );
    }

    fn assert_serialized_json<T: Serialize>(value: &T, expected: Value) {
        assert_eq!(
            serde_json::to_value(value).expect("serialize contract"),
            expected
        );
    }
}
