use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Describes an application-level invalidation or stream lifecycle event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "app_event.ts")]
pub enum AppEvent {
    /// Confirms that the client owns the application stream and may mount the shell.
    Ready,
    /// Tells clients that the persisted session row should be queried again.
    SessionTitleUpdated { session_id: String },
}

/// Identifies the in-memory browser document or Desktop window requesting the app stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "app_event.ts")]
pub struct WatchAppEventsRequest {
    pub client_instance_id: String,
}

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

    /// Verifies client ownership uses the dedicated in-memory request field.
    #[test]
    fn serializes_watch_request_in_camel_case() {
        assert_eq!(
            serde_json::to_value(WatchAppEventsRequest {
                client_instance_id: "client-1".to_string(),
            })
            .expect("watch request serializes"),
            serde_json::json!({ "clientInstanceId": "client-1" }),
        );
    }
}
