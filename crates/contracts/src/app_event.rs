use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes an application-level invalidation or stream lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "app_event.ts")]
pub enum AppEvent {
    /// Confirms that the application stream is subscribed and may be consumed.
    Ready,
    /// Tells clients that the persisted session row should be queried again.
    SessionTitleUpdated { session_id: String },
    /// Tells clients that the cached plugin lifecycle snapshot should be queried again.
    PluginStatusChanged { plugin_id: String },
    /// Tells clients that a replaced agent process may expose a different model catalog.
    AgentModelsInvalidated { agent_ref: String },
}

/// Opens the application event stream without filtering or ownership metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "app_event.ts")]
pub struct WatchAppEventsRequest {}

/// Exports the application event contract family for the generated frontend package.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    AppEvent::export(config)?;
    WatchAppEventsRequest::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{AppEvent, WatchAppEventsRequest};
    use pretty_assertions::assert_eq;

    /// Verifies the stream lifecycle and invalidation variants retain their wire shape.
    #[test]
    fn serializes_app_event_wire_shape() {
        assert_eq!(
            serde_json::to_value(AppEvent::Ready).expect("ready serializes"),
            serde_json::json!({ "type": "ready" }),
        );
        assert_eq!(
            serde_json::to_value(AppEvent::PluginStatusChanged {
                plugin_id: "ora.example".to_string(),
            })
            .expect("plugin invalidation serializes"),
            serde_json::json!({
                "type": "plugin_status_changed",
                "plugin_id": "ora.example",
            }),
        );
        assert_eq!(
            serde_json::to_value(AppEvent::SessionTitleUpdated {
                session_id: "session-1".to_string(),
            })
            .expect("title invalidation serializes"),
            serde_json::json!({
                "type": "session_title_updated",
                "session_id": "session-1",
            }),
        );
    }

    /// Verifies the application stream request carries no browser ownership state.
    #[test]
    fn serializes_empty_watch_request() {
        assert_eq!(
            serde_json::to_value(WatchAppEventsRequest {}).expect("watch request serializes"),
            serde_json::json!({}),
        );
    }
}
