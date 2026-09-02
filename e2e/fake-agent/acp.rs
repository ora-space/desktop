//! Minimal but complete ACP agent behavior behind the plugin's `agent/acp` channel.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use agent_client_protocol_schema::v1::*;
use ora_plugin_protocol::{
    AgentModel, INTERNAL_ERROR_CODE, INVALID_PARAMS_CODE, JSON_RPC_VERSION, METHOD_NOT_FOUND_CODE,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

const MODEL_CONFIG_ID: &str = "model";

/// Journal of the session-lifecycle ACP calls this agent served, in order.
///
/// Written into the package root the host runs the plugin from, which is the only channel a test
/// has into what the agent was actually asked. Ordering is the assertion that matters: it is what
/// separates a session restored in place from one Ora had to rebuild.
const ACP_JOURNAL: &str = "acp_calls.txt";

/// Marker file that makes `session/load` fail, standing in for an agent that lost the session.
///
/// A file rather than an environment variable because the host gives a plugin process no
/// environment of its own, and a test must not mutate its own.
const LOAD_REFUSAL_MARKER: &str = "refuse_session_load";

/// One fake session retained for the life of the plugin process.
#[derive(Debug, Clone)]
struct FakeSession {
    cwd: PathBuf,
    additional_directories: Vec<PathBuf>,
    model: String,
    title: Option<String>,
    active: bool,
}

/// One successfully handled ACP request plus notifications that must precede its response.
struct AcpCallResult {
    notifications: Vec<Value>,
    result: Value,
}

/// A JSON-RPC failure rendered inside the opaque ACP channel.
struct AcpError {
    code: i64,
    message: String,
}

impl AcpError {
    /// Classifies malformed method parameters without terminating the plugin process.
    fn invalid_params(method: &str, error: impl std::fmt::Display) -> Self {
        Self {
            code: INVALID_PARAMS_CODE,
            message: format!("invalid {method} params: {error}"),
        }
    }

    /// Reports a request whose session identity is not known to this process.
    fn unknown_session(session_id: &str) -> Self {
        Self {
            code: -32004,
            message: format!("unknown session {session_id}"),
        }
    }
}

/// Stateful ACP peer that implements the capabilities advertised during initialization.
#[derive(Default)]
pub(super) struct FakeAcpAgent {
    sessions: BTreeMap<String, FakeSession>,
    next_session_id: u64,
}

impl FakeAcpAgent {
    /// Accepts one complete ACP JSON-RPC frame and returns frames to forward to the host.
    pub(super) fn handle_frame(&mut self, frame: Value) -> Vec<Value> {
        let Some(object) = frame.as_object() else {
            return Vec::new();
        };
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return Vec::new();
        };
        let params = object.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = object.get("id").cloned() else {
            self.handle_notification(method, params);
            return Vec::new();
        };

        match self.handle_request(method, params) {
            Ok(mut call) => {
                call.notifications.push(json!({
                    "jsonrpc": JSON_RPC_VERSION,
                    "id": id,
                    "result": call.result,
                }));
                call.notifications
            }
            Err(error) => vec![json!({
                "jsonrpc": JSON_RPC_VERSION,
                "id": id,
                "error": {
                    "code": error.code,
                    "message": error.message,
                },
            })],
        }
    }

    /// Dispatches every ACP request implemented by the fake agent.
    fn handle_request(&mut self, method: &str, params: Value) -> Result<AcpCallResult, AcpError> {
        if method == AGENT_METHOD_NAMES.initialize {
            let request: InitializeRequest = parse_params(method, params)?;
            let capabilities = AgentCapabilities::new()
                .load_session(true)
                .session_capabilities(
                    SessionCapabilities::new()
                        .list(SessionListCapabilities::new())
                        .delete(SessionDeleteCapabilities::new())
                        .close(SessionCloseCapabilities::new()),
                );
            return success(
                InitializeResponse::new(request.protocol_version)
                    .agent_capabilities(capabilities)
                    .agent_info(Implementation::new("ora-fake-agent", "1.0.0")),
            );
        }
        if method == AGENT_METHOD_NAMES.session_new {
            let request: NewSessionRequest = parse_params(method, params)?;
            self.next_session_id += 1;
            let session_id = format!("fake-session-{}", self.next_session_id);
            let model = default_model_id().to_string();
            self.sessions.insert(
                session_id.clone(),
                FakeSession {
                    cwd: request.cwd,
                    additional_directories: request.additional_directories,
                    model: model.clone(),
                    title: None,
                    active: true,
                },
            );
            record_acp_call(method, &session_id);
            return success(
                NewSessionResponse::new(session_id).config_options(config_options(&model)),
            );
        }
        if method == AGENT_METHOD_NAMES.session_load {
            let request: LoadSessionRequest = parse_params(method, params)?;
            let session_id = request.session_id.to_string();
            record_acp_call(method, &session_id);
            if Path::new(LOAD_REFUSAL_MARKER).exists() {
                return Err(AcpError::unknown_session(&session_id));
            }
            let session = self
                .sessions
                .entry(session_id)
                .or_insert_with(|| FakeSession {
                    cwd: request.cwd.clone(),
                    additional_directories: request.additional_directories.clone(),
                    model: default_model_id().to_string(),
                    title: None,
                    active: true,
                });
            session.cwd = request.cwd;
            session.additional_directories = request.additional_directories;
            session.active = true;
            return success(
                LoadSessionResponse::new().config_options(config_options(&session.model)),
            );
        }
        if method == AGENT_METHOD_NAMES.session_list {
            let request: ListSessionsRequest = parse_params(method, params)?;
            let sessions = self
                .sessions
                .iter()
                .filter(|(_, session)| request.cwd.as_ref().is_none_or(|cwd| cwd == &session.cwd))
                .map(|(session_id, session)| {
                    SessionInfo::new(session_id.clone(), session.cwd.clone())
                        .additional_directories(session.additional_directories.clone())
                        .title(session.title.clone())
                })
                .collect();
            return success(ListSessionsResponse::new(sessions));
        }
        if method == AGENT_METHOD_NAMES.session_close {
            let request: CloseSessionRequest = parse_params(method, params)?;
            let session_id = request.session_id.to_string();
            let session = self
                .sessions
                .get_mut(&session_id)
                .ok_or_else(|| AcpError::unknown_session(&session_id))?;
            session.active = false;
            return success(CloseSessionResponse::new());
        }
        if method == AGENT_METHOD_NAMES.session_delete {
            let request: DeleteSessionRequest = parse_params(method, params)?;
            let session_id = request.session_id.to_string();
            if self.sessions.remove(&session_id).is_none() {
                return Err(AcpError::unknown_session(&session_id));
            }
            return success(DeleteSessionResponse::new());
        }
        if method == AGENT_METHOD_NAMES.session_set_config_option {
            let request: SetSessionConfigOptionRequest = parse_params(method, params)?;
            let session_id = request.session_id.to_string();
            let model = request
                .value
                .as_value_id()
                .map(ToString::to_string)
                .ok_or_else(|| AcpError::invalid_params(method, "model must be a value id"))?;
            if request.config_id.to_string() != MODEL_CONFIG_ID
                || !models().iter().any(|candidate| candidate.id == model)
            {
                return Err(AcpError::invalid_params(
                    method,
                    format!("unsupported model {model}"),
                ));
            }
            let session = self
                .sessions
                .get_mut(&session_id)
                .ok_or_else(|| AcpError::unknown_session(&session_id))?;
            session.model = model;
            return success(SetSessionConfigOptionResponse::new(config_options(
                &session.model,
            )));
        }
        if method == AGENT_METHOD_NAMES.session_prompt {
            return self.prompt(method, params);
        }
        Err(AcpError {
            code: METHOD_NOT_FOUND_CODE,
            message: format!("unknown ACP method {method}"),
        })
    }

    /// Consumes supported ACP notifications; prompt cancellation is instantaneous in this fake.
    fn handle_notification(&mut self, method: &str, params: Value) {
        if method == AGENT_METHOD_NAMES.session_cancel {
            let _ = parse_params::<CancelNotification>(method, params);
        }
    }

    /// Streams one deterministic assistant message before completing the prompt turn.
    fn prompt(&mut self, method: &str, params: Value) -> Result<AcpCallResult, AcpError> {
        let request: PromptRequest = parse_params(method, params)?;
        let session_id = request.session_id.to_string();
        record_acp_call(method, &session_id);
        let session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| AcpError::unknown_session(&session_id))?;
        if !session.active {
            return Err(AcpError::invalid_params(method, "session is closed"));
        }

        let prompt = request
            .prompt
            .iter()
            .filter_map(|block| {
                if let ContentBlock::Text(text) = block {
                    Some(text.text.as_str())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let response = if prompt.trim().is_empty() {
            "Fake agent completed the prompt.".to_string()
        } else {
            format!("Fake agent received: {prompt}")
        };
        if session.title.is_none() {
            let title = prompt
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("Fake agent session")
                .chars()
                .take(80)
                .collect();
            session.title = Some(title);
        }
        let notification = SessionNotification::new(
            request.session_id,
            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                TextContent::new(response),
            ))),
        );
        Ok(AcpCallResult {
            notifications: vec![json!({
                "jsonrpc": JSON_RPC_VERSION,
                "method": CLIENT_METHOD_NAMES.session_update,
                "params": notification,
            })],
            result: serde_json::to_value(PromptResponse::new(StopReason::EndTurn))
                .map_err(|error| AcpError::invalid_params(method, error))?,
        })
    }
}

/// Models exposed through both plugin discovery and each ACP session's model selector.
pub(super) fn models() -> Vec<AgentModel> {
    [
        ("anthropic/claude-sonnet-4", "claude-sonnet-4"),
        ("anthropic/claude-opus-4", "claude-opus-4"),
        ("openai/gpt-5", "gpt-5"),
        ("google/gemini-2.5-pro", "gemini-2.5-pro"),
        ("deepseek/deepseek-chat", "deepseek-chat"),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (id, display_name))| AgentModel {
        id: id.to_string(),
        display_name: display_name.to_string(),
        default: index == 0,
    })
    .collect()
}

/// Builds the ACP model selector with the current session value highlighted.
fn config_options(current_model: &str) -> Vec<SessionConfigOption> {
    let choices = models()
        .into_iter()
        .map(|model| SessionConfigSelectOption::new(model.id, model.display_name))
        .collect::<Vec<_>>();
    vec![
        SessionConfigOption::select(MODEL_CONFIG_ID, "Model", current_model.to_string(), choices)
            .category(SessionConfigOptionCategory::Model),
    ]
}

/// Appends one served session call to the journal beside this package.
fn record_acp_call(method: &str, session_id: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(ACP_JOURNAL)
    {
        let _ = writeln!(file, "{method} {session_id}");
    }
}

/// Returns the one model marked as the discovery default.
fn default_model_id() -> &'static str {
    "anthropic/claude-sonnet-4"
}

/// Deserializes method parameters while preserving the method in diagnostics.
fn parse_params<Params: DeserializeOwned>(method: &str, params: Value) -> Result<Params, AcpError> {
    serde_json::from_value(params).map_err(|error| AcpError::invalid_params(method, error))
}

/// Serializes a typed ACP response with no preceding notifications.
fn success<Response: Serialize>(response: Response) -> Result<AcpCallResult, AcpError> {
    let result = serde_json::to_value(response).map_err(|error| AcpError {
        code: INTERNAL_ERROR_CODE,
        message: format!("failed to encode ACP response: {error}"),
    })?;
    Ok(AcpCallResult {
        notifications: Vec::new(),
        result,
    })
}
