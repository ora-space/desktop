//! The workbench bridge: `plugin_webview_invoke` carries one method call from a workbench page to
//! its own plugin process and returns the answer.
//!
//! Identity comes from the calling webview label, never from the request. The page names only a
//! method and its params; the host resolves the label to a live workbench instance, checks the
//! method against the effective allowlist (manifest `∩` current registration), wraps the params
//! in a host-owned envelope, and invokes the exact process generation the instance is bound to.

use crate::state::DesktopState;
use crate::surface::gateway::{SurfaceConnection, SurfacePluginGateway};
use crate::surface::service::SurfaceService;
use ora_logging::{ora_debug, ora_warn};
use ora_plugin_lifecycle::PluginCallError;
use ora_surface::SurfaceSource;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use tauri::{Runtime, State, Webview};

/// How long the bridge waits for an on-demand plugin start before failing the call.
const PROCESS_START_WAIT: Duration = Duration::from_secs(15);

/// Upper bound of one request or response payload, well below the 16 MiB frame limit so a page
/// cannot starve the plugin's protocol channel.
pub const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

/// Longest plugin error message relayed to the page.
const MAX_PLUGIN_MESSAGE_CHARS: usize = 1024;

/// One page-side bridge request: a method name and its params, nothing else.
///
/// The request deliberately has no field for plugin id, version, instance, or generation: those
/// are the caller's identity, which the host derives from the webview label. Accepting them here
/// would be a confused-deputy hole.
#[derive(Debug, Deserialize)]
pub struct WorkbenchInvokeRequest {
    method: String,
    #[serde(default)]
    params: Value,
}

/// Why a bridge call failed, as the page sees it.
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
    /// The caller is not a live workbench instance.
    SurfaceUnavailable,
    /// The request or response payload exceeds `MAX_PAYLOAD_BYTES`.
    PayloadTooLarge,
    /// The method is not in the plugin's static allowlist.
    MethodNotAllowed,
    /// The method is in the allowlist but the current generation did not register it.
    MethodUnavailable,
    /// The plugin process could not be started or reached.
    PluginUnavailable,
    /// The plugin did not answer within the call timeout.
    PluginCallTimedOut,
    /// Anything else; details are in the host log only.
    Internal,
}

impl BridgeError {
    fn host(code: HostErrorCode) -> Self {
        Self::Host { code }
    }
}

impl<G: SurfacePluginGateway, R: Runtime> SurfaceService<G, R> {
    /// Forwards one method call from the workbench webview `label` to its plugin and returns the
    /// answer.
    pub async fn workbench_invoke(
        &self,
        label: &str,
        request: WorkbenchInvokeRequest,
    ) -> Result<Value, BridgeError> {
        let record = self
            .registry
            .resolve_label(label)
            .filter(|record| matches!(record.definition.source, SurfaceSource::Workbench(_)))
            .ok_or_else(|| BridgeError::host(HostErrorCode::SurfaceUnavailable))?;
        let SurfaceSource::Workbench(workbench) = &record.definition.source else {
            return Err(BridgeError::host(HostErrorCode::SurfaceUnavailable));
        };
        // The static allowlist is checked before the process is touched: a method the manifest
        // never exposed must fail identically whether or not the plugin is running.
        if !workbench
            .declared_methods
            .iter()
            .any(|declared| declared.as_str() == request.method)
        {
            return Err(BridgeError::host(HostErrorCode::MethodNotAllowed));
        }
        if payload_size(&request.params) > MAX_PAYLOAD_BYTES {
            return Err(BridgeError::host(HostErrorCode::PayloadTooLarge));
        }

        let plugin_id = &record.definition.plugin_id;
        // Started on demand so a page that loads faster than the process only sees a slower first
        // call, not a failure.
        let connection = self
            .gateway
            .ensure_running(plugin_id, PROCESS_START_WAIT)
            .await
            .map_err(|error| {
                ora_warn!(message = "workbench call could not reach the plugin", plugin_id = %plugin_id, error = %error);
                BridgeError::host(HostErrorCode::PluginUnavailable)
            })?;
        // The instance is pinned to the first process generation it successfully reaches. A page
        // keeps state derived from that generation (registered methods, plugin-side session), so
        // when the process restarted underneath it, the stale instance is closed instead of
        // silently talking to the new generation; reopening yields a fresh instance.
        let generation = connection.key().0;
        match self
            .registry
            .bind_workbench_generation(record.instance, generation)
        {
            None => return Err(BridgeError::host(HostErrorCode::SurfaceUnavailable)),
            Some(bound) if bound != generation => {
                ora_warn!(
                    message =
                        "workbench instance outlived its plugin process generation; closing it",
                    label,
                    bound_generation = bound,
                    running_generation = generation,
                );
                let _ = self.close(record.instance);
                return Err(BridgeError::host(HostErrorCode::SurfaceUnavailable));
            }
            Some(_) => {}
        }
        // The effective set is the manifest allowlist intersected with what this generation
        // registered; a method the manifest allows but the running version did not implement is
        // rejected here rather than surfacing as a raw `MethodNotRegistered`.
        if !connection.registered_methods().contains(&request.method) {
            return Err(BridgeError::host(HostErrorCode::MethodUnavailable));
        }

        let envelope = json!({
            "surface": {
                "instance_id": record.instance.value(),
                "generation": generation,
            },
            "input": request.params,
        });
        ora_debug!(
            message = "workbench call forwarded",
            label,
            method = %request.method,
            generation,
        );
        let result = connection
            .invoke(&request.method, envelope)
            .await
            .map_err(map_call_error)?;
        if payload_size(&result) > MAX_PAYLOAD_BYTES {
            return Err(BridgeError::host(HostErrorCode::PayloadTooLarge));
        }
        Ok(result)
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
        PluginCallError::Timeout => BridgeError::host(HostErrorCode::PluginCallTimedOut),
        // The effective-set check already ran, so a `MethodNotRegistered` here means the process
        // changed under us; report it as unavailable rather than leaking the raw condition.
        PluginCallError::Unavailable | PluginCallError::MethodNotRegistered => {
            BridgeError::host(HostErrorCode::PluginUnavailable)
        }
        PluginCallError::Transport(reason) => {
            ora_warn!(message = "workbench call transport failed", reason);
            BridgeError::host(HostErrorCode::Internal)
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

/// The bridge command; the only command a `plugin-webview:*` webview may invoke. Identity comes
/// from the caller label.
#[tauri::command]
pub async fn plugin_webview_invoke(
    webview: Webview,
    state: State<'_, DesktopState>,
    request: WorkbenchInvokeRequest,
) -> Result<Value, BridgeError> {
    state
        .surfaces
        .workbench_invoke(webview.label(), request)
        .await
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
                serde_json::to_value(BridgeError::host(HostErrorCode::MethodNotAllowed))
                    .expect("serialize"),
                sanitize_message(&"x".repeat(2000)).len(),
            ),
            (
                json!({ "kind": "plugin", "code": -32602, "message": "bad type" }),
                json!({ "kind": "host", "code": "PLUGIN_CALL_TIMED_OUT" }),
                json!({ "kind": "host", "code": "METHOD_NOT_ALLOWED" }),
                1024,
            )
        );
    }
}
