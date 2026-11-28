use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Selects which Git layer should be rendered in the workspace review surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub enum WorkspaceDiffScope {
    Branch,
    Unstaged,
    Staged,
    Committed,
}

/// Selects the whole patch, a file list, or a single file patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "workspace-diff.ts")]
pub enum WorkspaceDiffSelection {
    Patch,
    Files,
    File {
        path: String,
        old_path: Option<String>,
    },
}

/// Classifies the Git change displayed for a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub enum WorkspaceDiffStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unmerged,
    Unknown,
}

/// Describes a changed path without loading patch text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct WorkspaceDiffFile {
    pub path: String,
    pub old_path: Option<String>,
    pub status: WorkspaceDiffStatus,
}

/// Identifies which workspace diff should be computed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct GetWorkspaceDiffRequest {
    pub workspace_id: String,
    pub scope: WorkspaceDiffScope,
    pub selection: WorkspaceDiffSelection,
}

/// Returns the selected content and the revisions needed to render it.
///
/// `base_commit_id` is absent when the workspace has no recorded baseline (a
/// project's main checkout, or a historical worktree whose creation commit was
/// never recorded) - only meaningful for the `Branch`/`Committed` scopes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct GetWorkspaceDiffResponse {
    pub base_commit_id: Option<String>,
    pub head_commit_id: String,
    #[serde(flatten)]
    pub content: WorkspaceDiffContent,
}

/// Carries only the fields associated with the requested diff selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "workspace-diff.ts")]
pub enum WorkspaceDiffContent {
    Patch {
        patch: String,
        context_limited: bool,
    },
    Files {
        files: Vec<WorkspaceDiffFile>,
    },
    File {
        patch: String,
    },
}

/// Commits only the currently staged change set in one workspace's checkout.
///
/// The caller must stage changes first (see `StageWorkspaceChanges`) because
/// this operation never modifies the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct CommitWorkspaceChangesRequest {
    pub workspace_id: String,
    pub message: String,
}

/// Returns the commit created from the workspace checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct CommitWorkspaceChangesResponse {
    pub commit_id: String,
    pub summary: String,
}

/// Pushes the current workspace branch to its default remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct PushWorkspaceBranchRequest {
    pub workspace_id: String,
}

/// Returns the branch and remote updated by a successful push.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct PushWorkspaceBranchResponse {
    pub branch_name: String,
    pub remote_name: String,
}

/// Identifies which workspace staging status should be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct GetWorkspaceStatusRequest {
    pub workspace_id: String,
}

/// Returns the structured per-file staging state of one workspace checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct GetWorkspaceStatusResponse {
    pub entries: Vec<WorkspaceStatusEntry>,
}

/// Represents one changed file's staging state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct WorkspaceStatusEntry {
    pub path: String,
    pub is_staged: bool,
    pub is_untracked: bool,
}

/// Stages selected changes in one workspace checkout.
///
/// An empty `paths` list stages every current change in the workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct StageWorkspaceChangesRequest {
    pub workspace_id: String,
    /// Repo-relative paths to stage; an empty list stages every current change.
    pub paths: Vec<String>,
}

/// Returns the paths that were staged in one workspace checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct StageWorkspaceChangesResponse {
    pub staged_paths: Vec<String>,
}

/// Unstages selected changes in one workspace checkout.
///
/// An empty `paths` list is a defensive no-op and unstages nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct UnstageWorkspaceChangesRequest {
    pub workspace_id: String,
    /// Repo-relative paths to unstage; an empty list unstages nothing.
    pub paths: Vec<String>,
}

/// Returns the paths that were unstaged in one workspace checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "workspace-diff.ts")]
pub struct UnstageWorkspaceChangesResponse {
    pub unstaged_paths: Vec<String>,
}

/// Exports every TypeScript binding owned by this module so the aggregate exporter can keep one call site per family.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    WorkspaceDiffScope::export(config)?;
    WorkspaceDiffSelection::export(config)?;
    WorkspaceDiffFile::export(config)?;
    WorkspaceDiffStatus::export(config)?;
    WorkspaceDiffContent::export(config)?;
    GetWorkspaceDiffRequest::export(config)?;
    GetWorkspaceDiffResponse::export(config)?;
    CommitWorkspaceChangesRequest::export(config)?;
    CommitWorkspaceChangesResponse::export(config)?;
    PushWorkspaceBranchRequest::export(config)?;
    PushWorkspaceBranchResponse::export(config)?;
    GetWorkspaceStatusRequest::export(config)?;
    GetWorkspaceStatusResponse::export(config)?;
    WorkspaceStatusEntry::export(config)?;
    StageWorkspaceChangesRequest::export(config)?;
    StageWorkspaceChangesResponse::export(config)?;
    UnstageWorkspaceChangesRequest::export(config)?;
    UnstageWorkspaceChangesResponse::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{GetWorkspaceDiffResponse, WorkspaceDiffContent};
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies workspace diff payloads use the camel-case shape consumed by generated clients.
    #[test]
    fn serializes_workspace_diff_contracts() {
        let response = GetWorkspaceDiffResponse {
            base_commit_id: Some("base".to_string()),
            head_commit_id: "head".to_string(),
            content: WorkspaceDiffContent::Patch {
                patch: "patch".to_string(),
                context_limited: false,
            },
        };

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "baseCommitId": "base",
                "headCommitId": "head",
                "kind": "patch",
                "patch": "patch",
                "contextLimited": false,
            })
        );
    }

    /// Verifies a workspace with no recorded baseline serializes the field as `null`.
    #[test]
    fn serializes_missing_baseline_as_null_in_workspace_diff_contract() {
        let response = GetWorkspaceDiffResponse {
            base_commit_id: None,
            head_commit_id: "head".to_string(),
            content: WorkspaceDiffContent::Patch {
                patch: "patch".to_string(),
                context_limited: false,
            },
        };

        assert_eq!(
            serde_json::to_value(response).unwrap(),
            json!({
                "baseCommitId": null,
                "headCommitId": "head",
                "kind": "patch",
                "patch": "patch",
                "contextLimited": false,
            })
        );
    }
}
