use crate::definition::MountTarget;
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
        surface_id: String,
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
        file_name: String,
    },
    DownloadCompleted {
        instance: u64,
        plugin_id: String,
        file_name: String,
        path: String,
    },
    DownloadFailed {
        instance: u64,
        plugin_id: String,
        file_name: String,
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::SurfaceEvent;
    use crate::definition::MountTarget;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Pins the wire shape consumed by the hand-written frontend types.
    #[test]
    fn serializes_with_camel_case_tag_and_fields() {
        let events = [
            SurfaceEvent::Opened {
                instance: 7,
                plugin_id: "ora-space.skillhub".to_owned(),
                surface_id: "market".to_owned(),
                target: MountTarget::Windowed,
                title: "SkillHub".to_owned(),
            },
            SurfaceEvent::MigrateFailed {
                instance: 7,
                reason: "boom".to_owned(),
            },
            SurfaceEvent::DownloadCompleted {
                instance: 7,
                plugin_id: "ora-space.skillhub".to_owned(),
                file_name: "skill.zip".to_owned(),
                path: "/downloads/skill.zip".to_owned(),
            },
        ];
        assert_eq!(
            events.map(|event| serde_json::to_value(event).expect("serialize")),
            [
                json!({
                    "type": "opened",
                    "instance": 7,
                    "pluginId": "ora-space.skillhub",
                    "surfaceId": "market",
                    "target": "windowed",
                    "title": "SkillHub",
                }),
                json!({ "type": "migrateFailed", "instance": 7, "reason": "boom" }),
                json!({
                    "type": "downloadCompleted",
                    "instance": 7,
                    "pluginId": "ora-space.skillhub",
                    "fileName": "skill.zip",
                    "path": "/downloads/skill.zip",
                }),
            ]
        );
    }
}
