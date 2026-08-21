//! The panel bridge: `surface_request` carries one opaque payload from a panel page to its
//! plugin process (`ui/request`) and returns the process's answer.

use crate::state::DesktopState;
use crate::surface::gateway::{SurfaceConnection, SurfacePluginGateway};
use crate::surface::plugin_link::PROCESS_START_WAIT;
use crate::surface::service::SurfaceService;
use ora_logging::{ora_debug, ora_warn};
use ora_plugin_lifecycle::PluginCallError;
use ora_surface::{DownloadClock, SurfaceSource};
use serde::Serialize;
use serde_json::{Value, json};
use tauri::{Runtime, State, Webview};

/// JavaScript injected into every panel webview; defines `acquireOraSurfaceApi()`.
pub const PANEL_API_SCRIPT: &str = include_str!("panel_api.js");

/// Method the host invokes on the plugin for each bridge request.
pub const UI_REQUEST_METHOD: &str = "ui/request";

/// Upper bound of one request or response payload, well below the 16 MiB frame limit so a
/// page cannot starve the plugin's protocol channel.
pub const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

/// Longest plugin error message relayed to the page.
const MAX_PLUGIN_MESSAGE_CHARS: usize = 1024;

/// Why a bridge request failed, as the page sees it.
///
/// Host failures and plugin failures are different variants so a plugin's own error codes can
/// never impersonate a host condition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BridgeError {
    Host { code: HostErrorCode },
    Plugin { code: i64, message: String },
}

/// Host-side failure codes of the bridge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HostErrorCode {
    /// The caller is not a live panel instance.
    SurfaceClosed,
    /// The request or response payload exceeds `MAX_PAYLOAD_BYTES`.
    PayloadTooLarge,
    /// The plugin process could not be started or reached.
    PluginUnavailable,
    /// The plugin did not answer within the call timeout.
    Timeout,
    /// Anything else; details are in the host log only.
    Internal,
}

impl<G: SurfacePluginGateway, R: Runtime, C: DownloadClock + Send + Sync + 'static>
    SurfaceService<G, R, C>
{
    /// Forwards one payload from the panel webview `label` to its plugin and returns the answer.
    pub async fn request(&self, label: &str, payload: Value) -> Result<Value, BridgeError> {
        let record = self
            .registry
            .resolve_label(label)
            .filter(|record| matches!(record.definition.source, SurfaceSource::Panel(_)))
            .ok_or(BridgeError::Host {
                code: HostErrorCode::SurfaceClosed,
            })?;
        if payload_size(&payload) > MAX_PAYLOAD_BYTES {
            return Err(BridgeError::Host {
                code: HostErrorCode::PayloadTooLarge,
            });
        }
        let plugin_id = &record.definition.id.plugin_id;
        // The process is started on demand so a page that loads faster than the process only
        // sees a slower first request, not a failure.
        let connection = self
            .gateway
            .ensure_running(plugin_id, PROCESS_START_WAIT)
            .await
            .map_err(|error| {
                ora_warn!(message = "panel request could not reach the plugin", plugin_id = %plugin_id, error = %error);
                BridgeError::Host {
                    code: HostErrorCode::PluginUnavailable,
                }
            })?;
        let params = json!({
            "surfaceId": record.definition.id.surface_id.as_str(),
            "instanceId": record.instance.value(),
            "generation": connection.generation().0,
            "payload": payload,
        });
        ora_debug!(
            message = "panel request forwarded",
            label,
            generation = connection.generation().0
        );
        let result = connection
            .invoke(UI_REQUEST_METHOD, params)
            .await
            .map_err(map_call_error)?;
        let answer = match result {
            Value::Object(mut object) => object.remove("payload").unwrap_or(Value::Null),
            Value::Null
            | Value::Bool(_)
            | Value::Number(_)
            | Value::String(_)
            | Value::Array(_) => {
                return Err(BridgeError::Host {
                    code: HostErrorCode::Internal,
                });
            }
        };
        if payload_size(&answer) > MAX_PAYLOAD_BYTES {
            return Err(BridgeError::Host {
                code: HostErrorCode::PayloadTooLarge,
            });
        }
        Ok(answer)
    }
}

/// Serialized size of a JSON value, the unit the payload limit is defined in.
fn payload_size(value: &Value) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len())
        .unwrap_or(usize::MAX)
}

/// Maps a plugin call failure onto the page-facing error union.
fn map_call_error(error: PluginCallError) -> BridgeError {
    match error {
        PluginCallError::Remote { code, message } => BridgeError::Plugin {
            code,
            message: sanitize_message(&message),
        },
        PluginCallError::Timeout => BridgeError::Host {
            code: HostErrorCode::Timeout,
        },
        PluginCallError::Unavailable | PluginCallError::MethodNotRegistered => BridgeError::Host {
            code: HostErrorCode::PluginUnavailable,
        },
        PluginCallError::Transport(reason) => {
            ora_warn!(message = "panel request transport failed", reason);
            BridgeError::Host {
                code: HostErrorCode::Internal,
            }
        }
    }
}

/// Strips control characters and bounds the length of a plugin-authored message.
fn sanitize_message(message: &str) -> String {
    message
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_PLUGIN_MESSAGE_CHARS)
        .collect()
}

/// The bridge command used by `panel-surface:*` webviews; identity comes from the caller label.
#[tauri::command]
pub async fn surface_request(
    webview: Webview,
    state: State<'_, DesktopState>,
    payload: Value,
) -> Result<Value, BridgeError> {
    state.surfaces.request(webview.label(), payload).await
}

#[cfg(test)]
mod tests {
    use super::{BridgeError, HostErrorCode, map_call_error, sanitize_message};
    use ora_plugin_lifecycle::PluginCallError;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies the page-facing error union spelling and that plugin messages are sanitized.
    #[test]
    fn error_union_serializes_and_sanitizes() {
        let plugin = map_call_error(PluginCallError::Remote {
            code: -32602,
            message: "bad\u{7} type".to_owned(),
        });
        assert_eq!(
            (
                serde_json::to_value(&plugin).expect("serialize"),
                serde_json::to_value(map_call_error(PluginCallError::Timeout)).expect("serialize"),
                serde_json::to_value(BridgeError::Host {
                    code: HostErrorCode::PayloadTooLarge
                })
                .expect("serialize"),
                sanitize_message(&"x".repeat(2000)).len(),
            ),
            (
                json!({ "kind": "plugin", "code": -32602, "message": "bad type" }),
                json!({ "kind": "host", "code": "TIMEOUT" }),
                json!({ "kind": "host", "code": "PAYLOAD_TOO_LARGE" }),
                1024,
            )
        );
    }
}
