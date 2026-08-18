use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Distinguishes files from directories inside a task workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub enum WorkspaceEntryKind {
    File,
    Directory,
}

/// Describes one task-workspace entry using a slash-separated relative path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub struct WorkspaceEntry {
    pub name: String,
    pub path: String,
    pub kind: WorkspaceEntryKind,
    pub is_symbolic_link: bool,
}

/// Requests one immediate directory inside a task's managed worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub struct ListWorkspaceDirectoryRequest {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<String>,
}

/// Returns one normalized workspace directory and its immediate entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub struct ListWorkspaceDirectoryResponse {
    pub path: String,
    pub entries: Vec<WorkspaceEntry>,
}

/// Identifies one text file inside a task's managed worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub struct ReadWorkspaceFileRequest {
    pub task_id: String,
    pub path: String,
}

/// Returns one bounded UTF-8 file for the read-only workspace viewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub struct ReadWorkspaceFileResponse {
    pub path: String,
    pub content: String,
    pub version: String,
    pub size_bytes: u32,
}

/// Selects filename discovery or text-content search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub enum WorkspaceSearchKind {
    Files,
    Content,
}

/// Keeps filename results and line-oriented content matches structurally distinct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub enum WorkspaceSearchResult {
    File {
        path: String,
    },
    Match {
        path: String,
        line: u32,
        /// Uses ripgrep's one-based UTF-8 byte offset so every transport preserves its location.
        column: u32,
        #[serde(rename = "matchedText")]
        #[ts(rename = "matchedText")]
        matched_text: String,
        preview: String,
    },
}

/// Requests a bounded ripgrep search inside one task workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub struct SearchWorkspaceRequest {
    pub task_id: String,
    pub query: String,
    pub kind: WorkspaceSearchKind,
}

/// Returns ordered search results and indicates output truncated by the server limit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub struct SearchWorkspaceResponse {
    pub results: Vec<WorkspaceSearchResult>,
    pub truncated: bool,
}

/// Starts one workspace watcher stream scoped to a task's managed worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub struct WatchWorkspaceRequest {
    pub task_id: String,
}

/// Describes cache-invalidating changes emitted by the native workspace watcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub enum WorkspaceFileChange {
    Created { path: String },
    Modified { path: String },
    Removed { path: String },
    Renamed { from: String, path: String },
    RescanRequired,
}

/// Batches native filesystem changes so event storms do not trigger one refetch per callback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub struct WorkspaceFileEventBatch {
    pub changes: Vec<WorkspaceFileChange>,
}

/// Exports every filesystem and workspace viewer binding to the shared TypeScript package.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    WorkspaceEntryKind::export(config)?;
    WorkspaceEntry::export(config)?;
    ListWorkspaceDirectoryRequest::export(config)?;
    ListWorkspaceDirectoryResponse::export(config)?;
    ReadWorkspaceFileRequest::export(config)?;
    ReadWorkspaceFileResponse::export(config)?;
    WorkspaceSearchKind::export(config)?;
    WorkspaceSearchResult::export(config)?;
    SearchWorkspaceRequest::export(config)?;
    SearchWorkspaceResponse::export(config)?;
    WatchWorkspaceRequest::export(config)?;
    WorkspaceFileChange::export(config)?;
    WorkspaceFileEventBatch::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::WorkspaceSearchResult;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies content matches use the camel-case span field expected by browser clients.
    #[test]
    fn serializes_workspace_search_match() {
        let result = WorkspaceSearchResult::Match {
            path: "src/main.rs".to_string(),
            line: 7,
            column: 4,
            matched_text: "main".to_string(),
            preview: "fn main() {}".to_string(),
        };

        assert_eq!(
            serde_json::to_value(result)
                .unwrap_or_else(|error| panic!("serialize workspace search match: {error}")),
            json!({
                "kind": "match",
                "path": "src/main.rs",
                "line": 7,
                "column": 4,
                "matchedText": "main",
                "preview": "fn main() {}",
            })
        );
    }
}
