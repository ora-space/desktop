//! Routes `ui/push` notifications from plugin processes to the panel webview that owns the
//! session, numbering them per instance.

use crate::surface::gateway::{SurfaceConnection, SurfacePluginGateway};
use crate::surface::service::SurfaceService;
use ora_logging::{ora_debug, ora_warn};
use ora_plugin_lifecycle::InboundNotification;
use ora_surface::{DownloadClock, SurfaceInstanceId, SurfaceSource};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::Runtime;
use tokio::sync::broadcast::error::RecvError;

/// Notification a plugin emits to push one payload to a panel instance.
pub const UI_PUSH_METHOD: &str = "ui/push";

/// Page-side entry point the injected script defines; see `panel_api.js`.
const PUSH_ENTRY_POINT: &str = "window.__ORA_SURFACE_PUSH__";

/// Why one push was not delivered; returned so tests can assert on the exact filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PushRejection {
    NotPush,
    MalformedParams,
    UnknownInstance,
    SessionMismatch,
    NotPanel,
    StaleGeneration,
    WebviewMissing,
    EvalFailed,
}

/// Hands out the per-instance sequence numbers carried by push envelopes.
#[derive(Default)]
pub struct PushSequencer {
    next: Mutex<HashMap<SurfaceInstanceId, u64>>,
}

impl PushSequencer {
    /// Returns the next sequence number of `instance`, starting at 1.
    fn next(&self, instance: SurfaceInstanceId) -> u64 {
        let mut next = self
            .next
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let counter = next.entry(instance).or_insert(0);
        *counter += 1;
        *counter
    }

    /// Drops the counter of a closed instance.
    pub fn forget(&self, instance: SurfaceInstanceId) {
        self.next
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(&instance);
    }
}

impl<G: SurfacePluginGateway, R: Runtime, C: DownloadClock + Send + Sync + 'static>
    SurfaceService<G, R, C>
{
    /// Delivers one plugin notification to its panel, or explains why it was dropped.
    ///
    /// A push is only trusted when the named instance is a live panel of the emitting plugin and
    /// surface, and when the emitting process is the generation the host currently talks to;
    /// the last check is what keeps a restarted plugin's predecessor from writing into the page.
    pub fn deliver_push(&self, notification: &InboundNotification) -> Result<u64, PushRejection> {
        if notification.method != UI_PUSH_METHOD {
            return Err(PushRejection::NotPush);
        }
        let surface_id = notification.params.get("surfaceId").and_then(Value::as_str);
        let instance = notification
            .params
            .get("instanceId")
            .and_then(Value::as_u64);
        let (Some(surface_id), Some(instance)) = (surface_id, instance) else {
            return Err(PushRejection::MalformedParams);
        };
        let payload = notification
            .params
            .get("payload")
            .cloned()
            .unwrap_or(Value::Null);
        let instance = SurfaceInstanceId::new(instance);
        let record = self
            .registry
            .record(instance)
            .ok_or(PushRejection::UnknownInstance)?;
        if record.definition.id.plugin_id != notification.plugin_id
            || record.definition.id.surface_id.as_str() != surface_id
        {
            return Err(PushRejection::SessionMismatch);
        }
        if !matches!(record.definition.source, SurfaceSource::Panel(_)) {
            return Err(PushRejection::NotPanel);
        }
        let current = self
            .gateway
            .connection(&notification.plugin_id)
            .map(|connection| connection.generation())
            .ok();
        if current != Some(notification.generation) {
            return Err(PushRejection::StaleGeneration);
        }
        let webview = self
            .find_webview(record.label.as_str())
            .ok_or(PushRejection::WebviewMissing)?;
        let sequence = self.pushes.next(instance);
        let envelope = json!({ "sequence": sequence, "payload": payload });
        // The envelope is JSON, which is also a valid JavaScript expression, so no further
        // escaping is needed inside the call.
        webview
            .eval(format!("{PUSH_ENTRY_POINT}({envelope});"))
            .map_err(|error| {
                ora_warn!(message = "panel push eval failed", label = %record.label, error = %error);
                PushRejection::EvalFailed
            })?;
        ora_debug!(message = "panel push delivered", label = %record.label, sequence);
        Ok(sequence)
    }

    /// Consumes the gateway's notification stream until it closes, delivering every push.
    pub async fn route_pushes(&self) {
        let mut notifications = self.gateway.subscribe_notifications();
        loop {
            match notifications.recv().await {
                Ok(notification) => {
                    if let Err(rejection) = self.deliver_push(&notification)
                        && rejection != PushRejection::NotPush
                    {
                        ora_debug!(message = "plugin push dropped", plugin_id = %notification.plugin_id, rejection = ?rejection);
                    }
                }
                // Lost pushes are not replayed: the contract is best-effort and a page that needs
                // consistency re-reads its state from the plugin.
                Err(RecvError::Lagged(skipped)) => {
                    ora_warn!(message = "plugin push stream lagged", skipped);
                }
                Err(RecvError::Closed) => return,
            }
        }
    }
}
