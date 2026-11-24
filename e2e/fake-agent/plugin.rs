//! Ora plugin protocol shell around the in-process fake ACP agent.

use ora_plugin_protocol::{
    AGENT_ACP_METHOD, AGENT_LIST_MODELS_METHOD, AGENT_START_METHOD, AGENT_STOP_METHOD,
    AgentListModelsParams, AgentListModelsResult, AgentStartContext, AgentStartResult,
    INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE, JSON_RPC_VERSION, METHOD_NOT_FOUND_CODE,
    PluginRegistrationParams, REGISTER_METHOD, SHUTDOWN_METHOD, read_message, write_message,
};
use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use tokio::io::{stdin, stdout};

use super::acp::{FakeAcpAgent, models};

/// Journal of `agent/list_models` calls, written into the package root the host runs this from.
///
/// The host gives a plugin process no environment of its own and only sets the package root as
/// its working directory, so this file is the one channel a test can read the plugin's own view
/// of what it was asked. Recording every call is what lets a test assert both that discovery
/// happened and that connection startup did not perform it.
pub(super) const DISCOVERY_JOURNAL: &str = "list_models_calls.txt";
/// Marker file making `ora/register` declare legacy `effectResources` & `effect/*` methods.
const LEGACY_EFFECT_MARKER: &str = "register_legacy_effect_resources";
/// Journal written if the host ever invokes a retired `effect/*` method.
const EFFECT_CALLS_JOURNAL: &str = "effect_calls.txt";

/// Mutable process state shared by the outer plugin methods.
#[derive(Default)]
struct FakePlugin {
    running: bool,
    acp: FakeAcpAgent,
}

/// One outer JSON-RPC method failure.
struct PluginError {
    code: i64,
    message: String,
}

/// Runs the fake as a framed stdio plugin until the host sends `ora/shutdown` or closes stdin.
pub(super) async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut input = stdin();
    let mut output = stdout();
    let legacy_effects = Path::new(LEGACY_EFFECT_MARKER).exists();
    let mut methods = vec![
        AGENT_START_METHOD.to_string(),
        AGENT_STOP_METHOD.to_string(),
        AGENT_LIST_MODELS_METHOD.to_string(),
    ];
    if legacy_effects {
        methods.extend([
            "effect/coordinate".to_string(),
            "effect/reactivate".to_string(),
            "effect/verify_ready".to_string(),
        ]);
    }
    let registration = PluginRegistrationParams {
        methods,
        emits: vec![AGENT_ACP_METHOD.to_string()],
    };
    let mut params = serde_json::to_value(&registration)?;
    if legacy_effects && let Some(object) = params.as_object_mut() {
        object.insert(
            "effectResources".to_string(),
            json!([{
                "workspaceRelativePath": ".claude/skills",
                "materializationFormat": "ora/skill-directory.v1",
                "coordination": "quiesce_before_mutation"
            }]),
        );
    }
    write_message(
        &mut output,
        &json!({
            "jsonrpc": JSON_RPC_VERSION,
            "method": REGISTER_METHOD,
            "params": params,
        }),
    )
    .await?;

    let mut plugin = FakePlugin::default();
    while let Some(message) = read_message(&mut input).await? {
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            continue;
        };
        if method == SHUTDOWN_METHOD {
            break;
        }
        if let Some(id) = message.get("id").cloned() {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            let response = match plugin.handle_request(method, params) {
                Ok(result) => json!({
                    "jsonrpc": JSON_RPC_VERSION,
                    "id": id,
                    "result": result,
                }),
                Err(error) => json!({
                    "jsonrpc": JSON_RPC_VERSION,
                    "id": id,
                    "error": { "code": error.code, "message": error.message },
                }),
            };
            write_message(&mut output, &response).await?;
        } else if method == AGENT_ACP_METHOD && plugin.running {
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            for frame in plugin.acp.handle_frame(params) {
                write_message(
                    &mut output,
                    &json!({
                        "jsonrpc": JSON_RPC_VERSION,
                        "method": AGENT_ACP_METHOD,
                        "params": frame,
                    }),
                )
                .await?;
            }
        }
    }
    Ok(())
}

impl FakePlugin {
    /// Serves every registered outer plugin method using shared protocol DTOs.
    fn handle_request(&mut self, method: &str, params: Value) -> Result<Value, PluginError> {
        match method {
            AGENT_START_METHOD => {
                let context: AgentStartContext = serde_json::from_value(params)
                    .map_err(|error| PluginError::invalid_params(method, error))?;
                if context.cwd.as_os_str().is_empty() {
                    return Err(PluginError::invalid_params(method, "cwd must not be empty"));
                }
                self.running = true;
                serde_json::to_value(AgentStartResult::acp_v1()).map_err(PluginError::internal)
            }
            AGENT_STOP_METHOD => {
                self.running = false;
                Ok(json!({}))
            }
            AGENT_LIST_MODELS_METHOD => {
                let context: AgentListModelsParams = serde_json::from_value(params)
                    .map_err(|error| PluginError::invalid_params(method, error))?;
                // Discovery without a directory is meaningless: a real plugin resolves the agent's
                // per-project configuration from it, so an absent one must fail loudly here rather
                // than answer with a catalog that belongs to nowhere.
                if context.cwd.as_os_str().is_empty() {
                    return Err(PluginError::invalid_params(method, "cwd must not be empty"));
                }
                record_discovery(&context.cwd.to_string_lossy());
                serde_json::to_value(AgentListModelsResult { models: models() })
                    .map_err(PluginError::internal)
            }
            _ => {
                if method.starts_with("effect/") {
                    record_effect_call(method);
                }
                Err(PluginError {
                    code: METHOD_NOT_FOUND_CODE,
                    message: format!("unknown plugin method {method}"),
                })
            }
        }
    }
}

impl PluginError {
    /// Builds the standard invalid-parameters response for one outer method.
    fn invalid_params(method: &str, error: impl std::fmt::Display) -> Self {
        Self {
            code: INVALID_PARAMS_CODE,
            message: format!("invalid {method} params: {error}"),
        }
    }

    /// Converts an unexpected serialization failure into a JSON-RPC internal error.
    fn internal(error: serde_json::Error) -> Self {
        Self {
            code: INTERNAL_ERROR_CODE,
            message: error.to_string(),
        }
    }
}

/// Appends one discovery call's directory to the journal beside this package.
fn record_discovery(cwd: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(DISCOVERY_JOURNAL)
    {
        let _ = writeln!(file, "{cwd}");
    }
}

/// Appends any unexpected retired `effect/*` call to a journal beside this package.
fn record_effect_call(method: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(EFFECT_CALLS_JOURNAL)
    {
        let _ = writeln!(file, "{method}");
    }
}
