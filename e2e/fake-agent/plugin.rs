//! Ora plugin protocol shell around the in-process fake ACP agent.

use ora_plugin_protocol::{
    AGENT_ACP_METHOD, AGENT_LIST_MODELS_METHOD, AGENT_START_METHOD, AGENT_STOP_METHOD,
    AgentEffectCoordinationContext, AgentEffectReadinessContext, AgentListModelsParams,
    AgentListModelsResult, AgentStartContext, AgentStartResult, EFFECT_COORDINATE_METHOD,
    EFFECT_REACTIVATE_METHOD, EFFECT_VERIFY_READY_METHOD, INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE,
    JSON_RPC_VERSION, METHOD_NOT_FOUND_CODE, PluginEffectCoordination, PluginEffectResource,
    PluginRegistrationParams, REGISTER_METHOD, SHUTDOWN_METHOD, SKILL_DIRECTORY_V1, read_message,
    write_message,
};
use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::Write;
use tokio::io::{stdin, stdout};

use super::acp::{FakeAcpAgent, models};

/// Journal of `agent/list_models` calls, written into the package root the host runs this from.
///
/// The host gives a plugin process no environment of its own and only sets the package root as
/// its working directory, so this file is the one channel a test can read the plugin's own view
/// of what it was asked. Recording every call is what lets a test assert both that discovery
/// happened and that connection startup did not perform it.
pub(super) const DISCOVERY_JOURNAL: &str = "list_models_calls.txt";

/// Mutable process state shared by the outer plugin methods.
#[derive(Default)]
struct FakePlugin {
    running: bool,
    quiesced: bool,
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
    let registration = PluginRegistrationParams {
        methods: vec![
            AGENT_START_METHOD.to_string(),
            AGENT_STOP_METHOD.to_string(),
            AGENT_LIST_MODELS_METHOD.to_string(),
            EFFECT_COORDINATE_METHOD.to_string(),
            EFFECT_REACTIVATE_METHOD.to_string(),
            EFFECT_VERIFY_READY_METHOD.to_string(),
        ],
        emits: vec![AGENT_ACP_METHOD.to_string()],
        effect_resources: Some(vec![PluginEffectResource {
            workspace_relative_path: ".opencode/skills".to_string(),
            materialization_format: SKILL_DIRECTORY_V1.to_string(),
            coordination: PluginEffectCoordination::QuiesceBeforeMutation,
        }]),
    };
    write_message(
        &mut output,
        &json!({
            "jsonrpc": JSON_RPC_VERSION,
            "method": REGISTER_METHOD,
            "params": registration,
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
                self.quiesced = false;
                serde_json::to_value(AgentStartResult::acp_v1()).map_err(PluginError::internal)
            }
            AGENT_STOP_METHOD => {
                self.running = false;
                self.quiesced = false;
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
            EFFECT_COORDINATE_METHOD => {
                let context: AgentEffectCoordinationContext = serde_json::from_value(params)
                    .map_err(|error| PluginError::invalid_params(method, error))?;
                self.require_running()?;
                self.quiesced = true;
                Ok(json!({
                    "targetId": context.target_id,
                    "state": "safe_to_mutate",
                }))
            }
            EFFECT_REACTIVATE_METHOD => {
                let context: AgentEffectCoordinationContext = serde_json::from_value(params)
                    .map_err(|error| PluginError::invalid_params(method, error))?;
                self.require_running()?;
                self.quiesced = false;
                Ok(json!({
                    "targetId": context.target_id,
                    "state": "reactivated",
                }))
            }
            EFFECT_VERIFY_READY_METHOD => {
                let context: AgentEffectReadinessContext = serde_json::from_value(params)
                    .map_err(|error| PluginError::invalid_params(method, error))?;
                self.require_running()?;
                if self.quiesced {
                    return Err(PluginError {
                        code: -32000,
                        message: "fake agent is quiesced for a Skill mutation".to_string(),
                    });
                }
                serde_json::to_value(context).map_err(PluginError::internal)
            }
            _ => Err(PluginError {
                code: METHOD_NOT_FOUND_CODE,
                message: format!("unknown plugin method {method}"),
            }),
        }
    }

    /// Refuses Effect readiness operations until the outer agent has started.
    fn require_running(&self) -> Result<(), PluginError> {
        if self.running {
            Ok(())
        } else {
            Err(PluginError {
                code: -32000,
                message: "fake agent is not running".to_string(),
            })
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
