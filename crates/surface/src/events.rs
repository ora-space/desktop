use crate::definition::{MountTarget, SurfaceKind};
use serde::Serialize;

/// Projection of surface lifecycle and download progress for the frontend.
///
/// The desktop adapter emits these verbatim on `surface://event`; the TypeScript side mirrors the
/// shape by hand, so every rename here is a frontend contract change.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SurfaceEvent {
    Opened {
        instance: u64,
        plugin_id: String,
        kind: SurfaceKind,
        target: MountTarget,
        title: String,
    },
    Migrated {
        instance: u64,
        target: MountTarget,
    },
    MigrateFailed {
        instance: u64,
        reason: String,
    },
    Failed {
        instance: u64,
        reason: String,
    },
    Closed {
        instance: u64,
    },
    DownloadStarted {
        instance: u64,
        plugin_id: String,
        download_id: u64,
        file_name: String,
    },
    /// A prompt-disposition download landed; the trusted main webview must pick an action from
    /// `actions` and answer with `surface_resolve_download`.
    DownloadChoice {
        instance: u64,
        plugin_id: String,
        download_id: u64,
        page_origin: String,
        file_name: String,
        size_bytes: u64,
        actions: Vec<String>,
    },
    DownloadCompleted {
        instance: u64,
        plugin_id: String,
        download_id: u64,
        file_name: String,
        action: String,
        /// For a completed `import_skill` action: the prepared skill-import session the trusted
        /// main webview should open for review; `None` for actions without a follow-up.
        import_session_id: Option<String>,
    },
    DownloadFailed {
        instance: u64,
        plugin_id: String,
        download_id: u64,
        file_name: String,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::SurfaceEvent;
    use crate::definition::{MountTarget, SurfaceKind};
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Pins the wire shape consumed by the hand-written frontend types.
    #[test]
    fn serializes_with_camel_case_tag_and_fields() {
        let events = [
            SurfaceEvent::Opened {
                instance: 7,
                plugin_id: "acme.hub".to_owned(),
                kind: SurfaceKind::Webview,
                target: MountTarget::Windowed,
                title: "Example Hub".to_owned(),
            },
            SurfaceEvent::MigrateFailed {
                instance: 7,
                reason: "boom".to_owned(),
            },
            SurfaceEvent::DownloadCompleted {
                instance: 7,
                plugin_id: "acme.hub".to_owned(),
                download_id: 3,
                file_name: "skill.zip".to_owned(),
                action: "import_skill".to_owned(),
                import_session_id: Some("session-1".to_owned()),
            },
        ];
        assert_eq!(
            events.map(|event| serde_json::to_value(event).expect("serialize")),
            [
                json!({
                    "type": "opened",
                    "instance": 7,
                    "pluginId": "acme.hub",
                    "kind": "webview",
                    "target": "windowed",
                    "title": "Example Hub",
                }),
                json!({ "type": "migrateFailed", "instance": 7, "reason": "boom" }),
                json!({
                    "type": "downloadCompleted",
                    "instance": 7,
                    "pluginId": "acme.hub",
                    "downloadId": 3,
                    "fileName": "skill.zip",
                    "action": "import_skill",
                    "importSessionId": "session-1",
                }),
            ]
        );
    }
}
