use std::collections::HashSet;
use std::sync::Arc;

use serde_json::Value;

use crate::PluginRuntimeError;
use crate::host_requests::{HostRequestHandler, serve_request};
use crate::state::{ResponseRequest, RuntimeInner, RuntimeStatus};

pub(crate) const JSON_RPC_VERSION: &str = "2.0";
pub(crate) const REGISTER_METHOD: &str = "ora/register";
pub(crate) const SHUTDOWN_METHOD: &str = "ora/shutdown";

/// Holds the immutable capability declaration one plugin publishes during its handshake.
///
/// The two sets are deliberately separate because they describe opposite directions: `methods`
/// is what the host may invoke, `emits` is what the plugin may send unprompted. A method missing
/// from the matching set is a protocol violation rather than a silently ignored message, so a
/// plugin whose behaviour exceeds its declaration is rejected at the earliest possible moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PluginRegistration {
    pub methods: HashSet<String>,
    pub emits: HashSet<String>,
    /// Workspace-relative Effect surfaces this runtime consumes.
    pub effect_surfaces: Vec<PluginEffectSurface>,
}

/// Declares one filesystem Skill surface consumed by a plugin runtime.
///
/// The host supplies the Workspace root at materialization time. Plugins only declare a safe,
/// portable relative locator so a registration can be reused for every Workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginEffectSurface {
    pub workspace_relative_path: String,
    pub materialization_format: String,
    pub coordination: PluginEffectCoordination,
}

/// Selects the runtime boundary required before Ora mutates one declared surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginEffectCoordination {
    Uninterrupted,
    WaitForIdleAndRestart,
}

/// Carries one plugin-originated notification that passed the `emits` whitelist.
///
/// Notifications never carry a JSON-RPC id: the host does not answer them, and any correlation
/// they need belongs to the payload's own protocol (ACP frames carry their own ids).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginNotification {
    pub method: String,
    pub params: Value,
}

/// Applies one validated registration, notification, request, or response message to runtime
/// state, dispatching plugin requests to `host_requests`.
///
/// Returning `Err` means the connection can no longer be trusted and the caller must invalidate
/// the whole process; per-message recovery is intentionally absent because a host that guessed
/// at a malformed frame could silently mis-correlate every later response.
pub(crate) async fn handle_message<H: HostRequestHandler>(
    inner: &RuntimeInner,
    host_requests: &Arc<H>,
    message: Value,
) -> Result<(), String> {
    let object = message
        .as_object()
        .ok_or_else(|| "plugin message must be a JSON object".to_string())?;
    if object.get("jsonrpc").and_then(Value::as_str) != Some(JSON_RPC_VERSION) {
        return Err("plugin message has an invalid JSON-RPC version".to_string());
    }

    if let Some(method) = object.get("method").and_then(Value::as_str) {
        return handle_plugin_originated(inner, host_requests, object, method).await;
    }

    let request_id = object
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "plugin response has an invalid request ID".to_string())?;
    let result = match (object.get("result"), object.get("error")) {
        (Some(result), None) => Ok(result.clone()),
        (None, Some(error)) => {
            let code = error
                .get("code")
                .and_then(Value::as_i64)
                .ok_or_else(|| "plugin error response has an invalid code".to_string())?;
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .ok_or_else(|| "plugin error response has an invalid message".to_string())?;
            Err(PluginRuntimeError::Remote {
                code,
                message: message.to_string(),
            })
        }
        _ => return Err("plugin response must contain exactly one result or error".to_string()),
    };
    let response = inner.pending.lock().await.take_response(request_id);
    match response {
        ResponseRequest::Pending(sender) => {
            let _ = sender.send(result);
            Ok(())
        }
        // A timeout is local cancellation, not proof of a broken plugin. Its response remains a
        // well-formed answer to a request this generation sent, so it is safe to discard.
        ResponseRequest::Abandoned => Ok(()),
        ResponseRequest::Unmatched => Err(format!(
            "plugin responded with unknown request ID {request_id}"
        )),
    }
}

/// Splits registration, host requests, and whitelisted notifications; rejects everything else.
async fn handle_plugin_originated<H: HostRequestHandler>(
    inner: &RuntimeInner,
    host_requests: &Arc<H>,
    object: &serde_json::Map<String, Value>,
    method: &str,
) -> Result<(), String> {
    if let Some(request_id) = object.get("id") {
        // Host requests are only accepted from a registered plugin: the handshake is the one
        // message that may precede everything else, and a request during it would let a plugin
        // act before its declaration is known.
        if !matches!(*inner.status_tx.borrow(), RuntimeStatus::Ready) {
            return Err(format!(
                "plugin sent request {method} before completing registration"
            ));
        }
        if !(request_id.is_number() || request_id.is_string()) {
            return Err(format!("plugin request {method} has an invalid request ID"));
        }
        // Each request runs on its own task so a slow host method cannot delay the responses to
        // the host's own calls that share this reader.
        tokio::spawn(serve_request(
            Arc::clone(host_requests),
            inner.writer_tx.clone(),
            request_id.clone(),
            method.to_string(),
            object.get("params").cloned().unwrap_or(Value::Null),
        ));
        return Ok(());
    }

    if method == REGISTER_METHOD {
        if !matches!(*inner.status_tx.borrow(), RuntimeStatus::Starting) {
            return Err("plugin registered methods more than once".to_string());
        }
        let params = object.get("params");
        let registration = PluginRegistration {
            methods: parse_method_list(params, "methods")?
                .ok_or_else(|| "plugin registration is missing a methods array".to_string())?,
            emits: parse_method_list(params, "emits")?.unwrap_or_default(),
            effect_surfaces: parse_effect_surfaces(params)?,
        };
        *inner.registration.write().await = registration;
        inner.status_tx.send_replace(RuntimeStatus::Ready);
        return Ok(());
    }

    if !inner.registration.read().await.emits.contains(method) {
        return Err(format!(
            "plugin sent notification {method} without declaring it in emits"
        ));
    }
    // A closed inbound receiver means the host stopped consuming this plugin's stream; that is a
    // host-side lifecycle decision, so dropping the notification must not fail the connection.
    if let Some(inbound) = inner.inbound.lock().await.as_ref() {
        let _ = inbound.send(PluginNotification {
            method: method.to_string(),
            params: object.get("params").cloned().unwrap_or(Value::Null),
        });
    }
    Ok(())
}

/// Parses Effect descriptors as a strict registration contract rather than accepting opaque JSON.
fn parse_effect_surfaces(params: Option<&Value>) -> Result<Vec<PluginEffectSurface>, String> {
    let Some(value) = params.and_then(|params| params.get("effectSurfaces")) else {
        return Ok(Vec::new());
    };
    let entries = value
        .as_array()
        .ok_or_else(|| "plugin registration field effectSurfaces must be an array".to_string())?;
    entries
        .iter()
        .map(|entry| {
            let object = entry.as_object().ok_or_else(|| {
                "plugin registration effectSurfaces entry must be an object".to_string()
            })?;
            let required_string = |field: &str| {
                object
                    .get(field)
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        format!("plugin registration effectSurfaces entry has invalid {field}")
                    })
            };
            let coordination = match required_string("coordination")?.as_str() {
                "uninterrupted" => PluginEffectCoordination::Uninterrupted,
                "wait_for_idle_and_restart" => PluginEffectCoordination::WaitForIdleAndRestart,
                value => {
                    return Err(format!(
                        "plugin registration effectSurfaces entry has unknown coordination {value}"
                    ));
                }
            };
            Ok(PluginEffectSurface {
                workspace_relative_path: required_string("workspaceRelativePath")?,
                materialization_format: required_string("materializationFormat")?,
                coordination,
            })
        })
        .collect()
}

/// Reads one optional registration array into a duplicate-free method set.
fn parse_method_list(
    params: Option<&Value>,
    field: &str,
) -> Result<Option<HashSet<String>>, String> {
    let Some(value) = params.and_then(|params| params.get(field)) else {
        return Ok(None);
    };
    let entries = value
        .as_array()
        .ok_or_else(|| format!("plugin registration field {field} must be an array"))?;
    let mut parsed = HashSet::with_capacity(entries.len());
    for entry in entries {
        let entry = entry
            .as_str()
            .filter(|entry| !entry.is_empty())
            .ok_or_else(|| {
                format!("plugin registration field {field} contains an invalid entry")
            })?;
        if !parsed.insert(entry.to_string()) {
            return Err(format!("plugin registered duplicate {field} entry {entry}"));
        }
    }
    Ok(Some(parsed))
}
