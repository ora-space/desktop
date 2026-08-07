use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Identifies which ref family decorates a repository graph commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub enum RepositoryRefKind {
    Local,
    Remote,
    Tag,
}

/// Identifies the project whose Git repository should be inspected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct GetRepositorySnapshotRequest {
    pub project_id: String,
}

/// Returns the repository metadata, refs, bounded history, and current worktree status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct GetRepositorySnapshotResponse {
    pub snapshot: RepositorySnapshot,
}

/// Identifies the project whose main checkout changes should be rendered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct GetRepositoryWorkingTreeDiffRequest {
    pub project_id: String,
}

/// Returns the main checkout revision and its bounded working-tree patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct GetRepositoryWorkingTreeDiffResponse {
    pub diff: RepositoryWorkingTreeDiff,
}

/// Identifies a local branch to create from the repository's current HEAD.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct CreateRepositoryBranchRequest {
    pub project_id: String,
    pub branch_name: String,
}

/// Returns the local branch created for the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct CreateRepositoryBranchResponse {
    pub branch: String,
}

/// Identifies a local branch to check out in the project's main worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct CheckoutRepositoryBranchRequest {
    pub project_id: String,
    pub branch_name: String,
}

/// Returns the branch selected in the project's main worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct CheckoutRepositoryBranchResponse {
    pub branch: String,
}

/// Identifies the project repository whose remote refs should be refreshed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct FetchRepositoryRequest {
    pub project_id: String,
}

/// Returns the refreshed graph and remote tracking state after fetching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct FetchRepositoryResponse {
    pub snapshot: RepositorySnapshot,
}

/// Selects how a fetched upstream should be integrated into the current branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub enum PullRepositoryStrategy {
    FastForwardOnly,
    Merge,
    Rebase,
}

/// Identifies the project repository whose current branch should be synchronized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct PullRepositoryRequest {
    pub project_id: String,
    pub strategy: PullRepositoryStrategy,
}

/// Identifies the integration operation that can remain active while conflicts are resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub enum RepositorySyncOperation {
    Merge,
    Rebase,
}

/// Describes the result of a repository synchronization attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub enum PullRepositoryOutcome {
    AlreadyUpToDate,
    FastForwarded,
    Merged,
    Rebased,
    Diverged { ahead: u32, behind: u32 },
    Conflicted { operation: RepositorySyncOperation },
}

/// Returns the pull outcome and refreshed graph/tracking state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct PullRepositoryResponse {
    pub outcome: PullRepositoryOutcome,
    pub snapshot: RepositorySnapshot,
}

/// Selects whether an active merge or rebase should continue or be rolled back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub enum RepositorySyncAction {
    Continue,
    Abort,
}

/// Identifies the project repository whose active synchronization should be resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct ResolveRepositorySyncRequest {
    pub project_id: String,
    pub action: RepositorySyncAction,
}

/// Describes the result of resolving an active synchronization operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub enum RepositorySyncOutcome {
    Completed,
    Aborted,
    Conflicted,
}

/// Returns the resolution result and refreshed graph/worktree state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct ResolveRepositorySyncResponse {
    pub outcome: RepositorySyncOutcome,
    pub snapshot: RepositorySnapshot,
}

/// Selects the Git side to use when resolving one unmerged path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub enum RepositoryConflictSide {
    Ours,
    Theirs,
}

/// Identifies one conflicted path in the project's main checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct ResolveRepositoryConflictRequest {
    pub project_id: String,
    pub path: String,
    pub side: RepositoryConflictSide,
}

/// Returns refreshed status after selecting and staging one conflict side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct ResolveRepositoryConflictResponse {
    pub working_tree: RepositoryWorkingTree,
}

/// Identifies the project's checked-out branch to publish to its default remote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct PushRepositoryBranchRequest {
    pub project_id: String,
}

/// Returns the published branch and the refreshed graph/tracking state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct PushRepositoryBranchResponse {
    pub branch_name: String,
    pub remote_name: String,
    pub snapshot: RepositorySnapshot,
}

/// Selects every pending change or an explicit set of repository-relative paths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "paths", rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub enum RepositoryChangeSelection {
    All,
    Paths(Vec<String>),
}

/// Identifies changes to stage in the project's main checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct StageRepositoryChangesRequest {
    pub project_id: String,
    pub selection: RepositoryChangeSelection,
}

/// Returns the refreshed main checkout status after staging changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct StageRepositoryChangesResponse {
    pub working_tree: RepositoryWorkingTree,
}

/// Identifies changes to remove from the index in the project's main checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct UnstageRepositoryChangesRequest {
    pub project_id: String,
    pub selection: RepositoryChangeSelection,
}

/// Returns the refreshed main checkout status after unstaging changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct UnstageRepositoryChangesResponse {
    pub working_tree: RepositoryWorkingTree,
}

/// Identifies the commit message for staged changes in the project's main checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct CommitRepositoryChangesRequest {
    pub project_id: String,
    pub message: String,
}

/// Returns the new commit and refreshed main checkout status after committing changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct CommitRepositoryChangesResponse {
    pub commit_id: String,
    pub summary: String,
    pub working_tree: RepositoryWorkingTree,
}

/// Contains only the patch needed to review the project's main checkout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct RepositoryWorkingTreeDiff {
    pub head_commit_id: Option<String>,
    pub patch: String,
}

/// Represents the first read-only repository graph slice consumed by the app shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct RepositorySnapshot {
    pub project_id: String,
    pub root_path: String,
    pub head_commit_id: Option<String>,
    pub current_branch: Option<String>,
    pub references: Vec<RepositoryReference>,
    pub commits: Vec<RepositoryCommit>,
    pub working_tree: RepositoryWorkingTree,
    pub remote_status: RepositoryRemoteStatus,
    pub sync_operation: Option<RepositorySyncOperation>,
}

/// Summarizes the current branch's configured upstream and commit distance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct RepositoryRemoteStatus {
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
}

/// Associates one visible ref with the commit it decorates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct RepositoryReference {
    pub name: String,
    pub commit_id: String,
    pub kind: RepositoryRefKind,
}

/// Contains the topology and author metadata needed for one graph row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct RepositoryCommit {
    pub id: String,
    pub short_id: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: String,
    pub reference_names: Vec<String>,
}

/// Summarizes the checked-out worktree without exposing raw porcelain output to the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct RepositoryWorkingTree {
    pub changed_files: u32,
    pub staged_files: u32,
    pub unstaged_files: u32,
    pub untracked_files: u32,
    pub conflicted_files: u32,
    pub files: Vec<RepositoryWorkingTreeFile>,
}

/// Describes one pending main-checkout path and which side of the index differs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct RepositoryWorkingTreeFile {
    pub path: String,
    pub status: String,
    pub staged: bool,
    pub unstaged: bool,
}

/// Identifies one commit detail request using a project-owned repository root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct GetRepositoryCommitRequest {
    pub project_id: String,
    pub commit_id: String,
}

/// Returns a selected commit and its changed paths without loading the full patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct GetRepositoryCommitResponse {
    pub commit: RepositoryCommitDetails,
}

/// Identifies the bounded patch requested after a commit file is opened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct GetRepositoryCommitDiffRequest {
    pub project_id: String,
    pub commit_id: String,
    pub parent_commit_id: Option<String>,
    pub path: String,
}

/// Returns one historical commit's bounded unified patch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct GetRepositoryCommitDiffResponse {
    pub patch: String,
}

/// Extends graph metadata with the files changed by the selected commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct RepositoryCommitDetails {
    pub id: String,
    pub short_id: String,
    pub parents: Vec<String>,
    pub subject: String,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: String,
    pub files: Vec<RepositoryCommitFile>,
}

/// Describes one changed path using Git's compact name-status code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "repository.ts")]
pub struct RepositoryCommitFile {
    pub status: String,
    pub path: String,
}

/// Exports every repository graph binding to the shared TypeScript package.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    RepositoryRefKind::export(config)?;
    CreateRepositoryBranchRequest::export(config)?;
    CreateRepositoryBranchResponse::export(config)?;
    CheckoutRepositoryBranchRequest::export(config)?;
    CheckoutRepositoryBranchResponse::export(config)?;
    RepositoryChangeSelection::export(config)?;
    FetchRepositoryRequest::export(config)?;
    FetchRepositoryResponse::export(config)?;
    PullRepositoryStrategy::export(config)?;
    PullRepositoryRequest::export(config)?;
    RepositorySyncOperation::export(config)?;
    PullRepositoryOutcome::export(config)?;
    PullRepositoryResponse::export(config)?;
    RepositorySyncAction::export(config)?;
    ResolveRepositorySyncRequest::export(config)?;
    RepositorySyncOutcome::export(config)?;
    ResolveRepositorySyncResponse::export(config)?;
    RepositoryConflictSide::export(config)?;
    ResolveRepositoryConflictRequest::export(config)?;
    ResolveRepositoryConflictResponse::export(config)?;
    PushRepositoryBranchRequest::export(config)?;
    PushRepositoryBranchResponse::export(config)?;
    StageRepositoryChangesRequest::export(config)?;
    StageRepositoryChangesResponse::export(config)?;
    UnstageRepositoryChangesRequest::export(config)?;
    UnstageRepositoryChangesResponse::export(config)?;
    CommitRepositoryChangesRequest::export(config)?;
    CommitRepositoryChangesResponse::export(config)?;
    GetRepositorySnapshotRequest::export(config)?;
    GetRepositorySnapshotResponse::export(config)?;
    GetRepositoryCommitDiffRequest::export(config)?;
    GetRepositoryCommitDiffResponse::export(config)?;
    GetRepositoryWorkingTreeDiffRequest::export(config)?;
    GetRepositoryWorkingTreeDiffResponse::export(config)?;
    RepositoryWorkingTreeDiff::export(config)?;
    RepositorySnapshot::export(config)?;
    RepositoryRemoteStatus::export(config)?;
    RepositoryReference::export(config)?;
    RepositoryCommit::export(config)?;
    RepositoryWorkingTree::export(config)?;
    RepositoryWorkingTreeFile::export(config)?;
    GetRepositoryCommitRequest::export(config)?;
    GetRepositoryCommitResponse::export(config)?;
    RepositoryCommitDetails::export(config)?;
    RepositoryCommitFile::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        GetRepositoryCommitRequest, PullRepositoryOutcome, RepositoryChangeSelection,
        RepositoryCommit, RepositoryRefKind, RepositoryReference, RepositoryRemoteStatus,
        RepositorySnapshot, RepositorySyncOperation, RepositoryWorkingTree,
        StageRepositoryChangesRequest,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies graph topology and ref metadata use the camelCase wire shape shared by adapters.
    #[test]
    fn serializes_repository_snapshot() {
        let snapshot = RepositorySnapshot {
            project_id: "project-1".to_string(),
            root_path: "/workspace/ora".to_string(),
            head_commit_id: Some("abc123".to_string()),
            current_branch: Some("main".to_string()),
            references: vec![RepositoryReference {
                name: "main".to_string(),
                commit_id: "abc123".to_string(),
                kind: RepositoryRefKind::Local,
            }],
            commits: vec![RepositoryCommit {
                id: "abc123".to_string(),
                short_id: "abc123".to_string(),
                parents: Vec::new(),
                subject: "initial".to_string(),
                author_name: "Ora Tests".to_string(),
                author_email: "ora@example.com".to_string(),
                authored_at: "2026-08-04T10:00:00+08:00".to_string(),
                reference_names: vec!["main".to_string()],
            }],
            remote_status: RepositoryRemoteStatus {
                upstream: Some("origin/main".to_string()),
                ahead: 1,
                behind: 2,
            },
            sync_operation: None,
            working_tree: RepositoryWorkingTree {
                changed_files: 1,
                staged_files: 1,
                unstaged_files: 0,
                untracked_files: 0,
                conflicted_files: 0,
                files: Vec::new(),
            },
        };

        assert_eq!(
            serde_json::to_value(&snapshot).expect("serialize snapshot"),
            json!({
                "projectId": "project-1",
                "rootPath": "/workspace/ora",
                "headCommitId": "abc123",
                "currentBranch": "main",
                "references": [{
                    "name": "main",
                    "commitId": "abc123",
                    "kind": "local",
                }],
                "commits": [{
                    "id": "abc123",
                    "shortId": "abc123",
                    "parents": [],
                    "subject": "initial",
                    "authorName": "Ora Tests",
                    "authorEmail": "ora@example.com",
                    "authoredAt": "2026-08-04T10:00:00+08:00",
                    "referenceNames": ["main"],
                }],
                "remoteStatus": {
                    "upstream": "origin/main",
                    "ahead": 1,
                    "behind": 2,
                },
                "syncOperation": null,
                "workingTree": {
                    "changedFiles": 1,
                    "stagedFiles": 1,
                    "unstagedFiles": 0,
                    "untrackedFiles": 0,
                    "conflictedFiles": 0,
                    "files": [],
                },
            })
        );
    }

    /// Verifies pull outcomes preserve enough structure for the UI to choose a safe resolution path.
    #[test]
    fn serializes_divergence_and_conflict_outcomes() {
        assert_eq!(
            serde_json::to_value(PullRepositoryOutcome::Diverged {
                ahead: 2,
                behind: 3,
            })
            .expect("serialize diverged outcome"),
            json!({
                "kind": "diverged",
                "ahead": 2,
                "behind": 3,
            })
        );
        assert_eq!(
            serde_json::to_value(PullRepositoryOutcome::Conflicted {
                operation: RepositorySyncOperation::Rebase,
            })
            .expect("serialize conflict outcome"),
            json!({
                "kind": "conflicted",
                "operation": "rebase",
            })
        );
    }

    /// Verifies commit detail requests keep both resource identifiers explicit on the wire.
    #[test]
    fn serializes_commit_detail_request() {
        assert_eq!(
            serde_json::to_value(GetRepositoryCommitRequest {
                project_id: "project-1".to_string(),
                commit_id: "abc123".to_string(),
            })
            .expect("serialize commit request"),
            json!({
                "projectId": "project-1",
                "commitId": "abc123",
            })
        );
    }

    /// Verifies explicit change selections remain transport-neutral and camelCase compatible.
    #[test]
    fn serializes_repository_stage_request() {
        assert_eq!(
            serde_json::to_value(StageRepositoryChangesRequest {
                project_id: "project-1".to_string(),
                selection: RepositoryChangeSelection::Paths(vec!["README.md".to_string()]),
            })
            .expect("serialize stage request"),
            json!({
                "projectId": "project-1",
                "selection": { "kind": "paths", "paths": ["README.md"] },
            })
        );
    }
}
