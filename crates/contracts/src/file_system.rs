use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Distinguishes selectable filesystem entries from entries whose metadata cannot be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub enum FileSystemEntryKind {
    File,
    Directory,
    Unavailable,
}

/// Describes one child entry returned by a server-side directory listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub struct FileSystemEntry {
    pub name: String,
    pub path: String,
    pub kind: FileSystemEntryKind,
    pub is_symbolic_link: bool,
}

/// Describes one server-derived ancestor used to navigate without parsing path separators in JavaScript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub struct FileSystemBreadcrumb {
    pub name: String,
    pub path: String,
}

/// Requests one server-side directory, defaulting to the server user's home when omitted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub struct ListDirectoryRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<String>,
}

/// Returns the resolved directory, its parent, and every visible child entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "file-system.ts")]
pub struct ListDirectoryResponse {
    pub current_path: String,
    pub parent_path: Option<String>,
    pub breadcrumbs: Vec<FileSystemBreadcrumb>,
    pub entries: Vec<FileSystemEntry>,
}

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
    FileSystemEntryKind::export(config)?;
    FileSystemEntry::export(config)?;
    FileSystemBreadcrumb::export(config)?;
    ListDirectoryRequest::export(config)?;
    ListDirectoryResponse::export(config)?;
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
    use super::{
        FileSystemBreadcrumb, FileSystemEntry, FileSystemEntryKind, ListDirectoryRequest,
        ListDirectoryResponse, WorkspaceSearchResult,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies filesystem contracts preserve the path and entry metadata consumed by the picker.
    #[test]
    fn serializes_file_system_contracts() {
        let request = ListDirectoryRequest {
            path: Some("/home/ora".to_string()),
        };
        let response = ListDirectoryResponse {
            current_path: "/home/ora".to_string(),
            parent_path: Some("/home".to_string()),
            breadcrumbs: vec![
                FileSystemBreadcrumb {
                    name: "/".to_string(),
                    path: "/".to_string(),
                },
                FileSystemBreadcrumb {
                    name: "ora".to_string(),
                    path: "/home/ora".to_string(),
                },
            ],
            entries: vec![FileSystemEntry {
                name: "project".to_string(),
                path: "/home/ora/project".to_string(),
                kind: FileSystemEntryKind::Directory,
                is_symbolic_link: true,
            }],
        };

        assert_eq!(
            serde_json::to_value(request)
                .unwrap_or_else(|error| panic!("serialize directory request: {error}")),
            json!({ "path": "/home/ora" })
        );
        assert_eq!(
            serde_json::to_value(ListDirectoryRequest::default())
                .unwrap_or_else(|error| panic!("serialize default directory request: {error}")),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(response)
                .unwrap_or_else(|error| panic!("serialize directory response: {error}")),
            json!({
                "currentPath": "/home/ora",
                "parentPath": "/home",
                "breadcrumbs": [
                    { "name": "/", "path": "/" },
                    { "name": "ora", "path": "/home/ora" },
                ],
                "entries": [{
                    "name": "project",
                    "path": "/home/ora/project",
                    "kind": "directory",
                    "isSymbolicLink": true,
                }],
            })
        );
    }

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
