use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Selects which Git layer should be rendered in the workspace review surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace_diff.ts")]
pub enum WorkspaceDiffScope {
    Branch,
    Unstaged,
    Staged,
    Committed,
}

/// Identifies which workspace diff should be computed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace_diff.ts")]
pub struct GetWorkspaceDiffRequest {
    pub workspace_id: String,
    pub scope: WorkspaceDiffScope,
}

/// Returns one standard unified patch and the revisions needed to render it.
///
/// `base_commit_id` is absent when the workspace has no recorded baseline (a
/// project's main checkout, or a historical worktree whose creation commit was
/// never recorded) — only meaningful for the `Branch`/`Committed` scopes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace_diff.ts")]
pub struct GetWorkspaceDiffResponse {
    pub base_commit_id: Option<String>,
    pub head_commit_id: String,
    pub patch: String,
}

/// Commits every current change in one workspace's checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace_diff.ts")]
pub struct CommitWorkspaceChangesRequest {
    pub workspace_id: String,
    pub message: String,
}

/// Returns the commit created from the workspace checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace_diff.ts")]
pub struct CommitWorkspaceChangesResponse {
    pub commit_id: String,
    pub summary: String,
}

/// Pushes the current workspace branch to its default remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace_diff.ts")]
pub struct PushWorkspaceBranchRequest {
    pub workspace_id: String,
}

/// Returns the branch and remote updated by a successful push.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace_diff.ts")]
pub struct PushWorkspaceBranchResponse {
    pub branch_name: String,
    pub remote_name: String,
}

/// Exports every TypeScript binding owned by this module so the aggregate exporter can keep one call site per family.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    WorkspaceDiffScope::export(config)?;
    GetWorkspaceDiffRequest::export(config)?;
    GetWorkspaceDiffResponse::export(config)?;
    CommitWorkspaceChangesRequest::export(config)?;
    CommitWorkspaceChangesResponse::export(config)?;
    PushWorkspaceBranchRequest::export(config)?;
    PushWorkspaceBranchResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::GetWorkspaceDiffResponse;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies workspace diff payloads use the camel-case shape consumed by generated clients.
    #[test]
    fn serializes_workspace_diff_contracts() {
        let response = GetWorkspaceDiffResponse {
            base_commit_id: Some("base".to_string()),
            head_commit_id: "head".to_string(),
            patch: "patch".to_string(),
        };

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "baseCommitId": "base",
                "headCommitId": "head",
                "patch": "patch",
            })
        );
    }

    /// Verifies a workspace with no recorded baseline serializes the field as `null`.
    #[test]
    fn serializes_missing_baseline_as_null_in_workspace_diff_contract() {
        let response = GetWorkspaceDiffResponse {
            base_commit_id: None,
            head_commit_id: "head".to_string(),
            patch: "patch".to_string(),
        };

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "baseCommitId": null,
                "headCommitId": "head",
                "patch": "patch",
            })
        );
    }
}
