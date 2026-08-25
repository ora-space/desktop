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

/// Describes the stable workspace identity used by Sessions and WorkflowRuns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace.ts")]
pub struct Workspace {
    pub id: String,
    pub project_id: String,
    pub kind: WorkspaceKind,
    pub lifecycle: WorkspaceLifecycle,
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
    Workspace::export(config)?;
    ListWorkspacesRequest::export(config)?;
    ListWorkspacesResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        ListWorkspacesRequest, ListWorkspacesResponse, Workspace, WorkspaceKind, WorkspaceLifecycle,
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
        };

        assert_serialized_json(
            &workspace,
            json!({
                "id": "workspace-1",
                "projectId": "project-1",
                "kind": "main",
                "lifecycle": "active",
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
                }],
            }),
        );
    }

    fn assert_serialized_json<T: Serialize>(value: &T, expected: Value) {
        assert_eq!(
            serde_json::to_value(value).expect("serialize contract"),
            expected
        );
    }
}
