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

#[cfg(test)]
mod tests {
    use super::{
        FileSystemBreadcrumb, FileSystemEntry, FileSystemEntryKind, ListDirectoryRequest,
        ListDirectoryResponse,
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
            serde_json::to_value(request).unwrap(),
            json!({ "path": "/home/ora" })
        );
        assert_eq!(
            serde_json::to_value(ListDirectoryRequest::default()).unwrap(),
            json!({})
        );
        assert_eq!(
            serde_json::to_value(response).unwrap(),
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
}
