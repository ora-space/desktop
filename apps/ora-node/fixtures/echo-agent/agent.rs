//! Plugin protocol shell and the minimal ACP agent inside it.

use agent_client_protocol_schema::v1::{
    AGENT_METHOD_NAMES, AgentCapabilities, CLIENT_METHOD_NAMES, CancelNotification,
    CloseSessionRequest, CloseSessionResponse, ContentBlock, ContentChunk, Implementation,
    InitializeRequest, InitializeResponse, NewSessionRequest, NewSessionResponse, PromptRequest,
    PromptResponse, SessionCapabilities, SessionCloseCapabilities, SessionNotification,
    SessionUpdate, StopReason, TextContent,
};
use ora_plugin_protocol::{
    AGENT_ACP_METHOD, AGENT_LIST_MODELS_METHOD, AGENT_START_METHOD, AGENT_STOP_METHOD,
    AgentListModelsResult, AgentStartResult, CHILDPROCESS_EXIT_METHOD, CHILDPROCESS_SPAWN_METHOD,
    JSON_RPC_VERSION, METHOD_NOT_FOUND_CODE, PluginRegistrationParams, REGISTER_METHOD,
    SHUTDOWN_METHOD, read_message, write_message,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use tokio::io::{Stdout, stdin, stdout};

/// Identity of the one host request this fixture sends.
const SPAWN_REQUEST_ID: &str = "echo-spawn";

/// Size of the `[large]` answer, above the 256 KiB a Thread record may hold.
const LARGE_ANSWER_BYTES: usize = 300 * 1024;

/// A prompt whose response is deferred until something else happens.
struct PendingPrompt {
    request_id: Value,
    session_id: String,
}

/// Everything the fixture remembers between frames.
#[derive(Default)]
struct EchoAgent {
    sessions: BTreeMap<String, PathBuf>,
    /// A `[hold]` prompt, answered when the host cancels it.
    held: Option<PendingPrompt>,
    /// A `[commit]` prompt, answered when the host-spawned commit exits.
    committing: Option<PendingPrompt>,
}

/// Runs until the host sends `ora/shutdown` or closes stdin.
pub(super) async fn run() -> Result<(), Box<dyn std::error::Error>> {
    record_pid();
    let mut input = stdin();
    let mut output = stdout();
    let registration = PluginRegistrationParams {
        methods: vec![
            AGENT_START_METHOD.to_string(),
            AGENT_STOP_METHOD.to_string(),
            AGENT_LIST_MODELS_METHOD.to_string(),
        ],
        emits: vec![AGENT_ACP_METHOD.to_string()],
        effect_resources: None,
    };
    write_message(
        &mut output,
        &json!({"jsonrpc": JSON_RPC_VERSION, "method": REGISTER_METHOD, "params": registration}),
    )
    .await?;
    let mut agent = EchoAgent::default();
    while let Some(message) = read_message(&mut input).await? {
        let method = message.get("method").and_then(Value::as_str);
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        match (method, message.get("id").cloned()) {
            (Some(SHUTDOWN_METHOD), _) => break,
            (Some(AGENT_ACP_METHOD), None) => agent.acp_frame(&mut output, params).await?,
            (Some(CHILDPROCESS_EXIT_METHOD), None) => agent.commit_finished(&mut output).await?,
            (Some(method), Some(id)) => {
                let result = match method {
                    AGENT_START_METHOD => serde_json::to_value(AgentStartResult::acp_v1())?,
                    AGENT_STOP_METHOD => json!({}),
                    AGENT_LIST_MODELS_METHOD => {
                        serde_json::to_value(AgentListModelsResult { models: Vec::new() })?
                    }
                    _ => {
                        let error = json!({"code": METHOD_NOT_FOUND_CODE, "message": method});
                        let reply = json!({"jsonrpc": JSON_RPC_VERSION, "id": id, "error": error});
                        write_message(&mut output, &reply).await?;
                        continue;
                    }
                };
                let reply = json!({"jsonrpc": JSON_RPC_VERSION, "id": id, "result": result});
                write_message(&mut output, &reply).await?;
            }
            // A failed spawn never produces an exit, so its error reply finishes the commit.
            (None, Some(id)) if id == SPAWN_REQUEST_ID && message.get("error").is_some() => {
                agent.commit_finished(&mut output).await?;
            }
            (Some(_), None) | (None, _) => {}
        }
    }
    Ok(())
}

impl EchoAgent {
    /// Serves one ACP frame from the host.
    async fn acp_frame(
        &mut self,
        output: &mut Stdout,
        frame: Value,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let method = frame
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let params = frame.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = frame.get("id").cloned() else {
            if method == AGENT_METHOD_NAMES.session_cancel {
                let cancel: CancelNotification = serde_json::from_value(params)?;
                if let Some(held) = self
                    .held
                    .take_if(|held| held.session_id == cancel.session_id.to_string())
                {
                    respond(
                        output,
                        held.request_id,
                        &PromptResponse::new(StopReason::Cancelled),
                    )
                    .await?;
                }
            }
            return Ok(());
        };
        if method == AGENT_METHOD_NAMES.initialize {
            let request: InitializeRequest = serde_json::from_value(params)?;
            let capabilities = AgentCapabilities::new().session_capabilities(
                SessionCapabilities::new().close(SessionCloseCapabilities::new()),
            );
            let response = InitializeResponse::new(request.protocol_version)
                .agent_capabilities(capabilities)
                .agent_info(Implementation::new("ora-node-echo-agent", "1.0.0"));
            return respond(output, id, &response).await;
        }
        if method == AGENT_METHOD_NAMES.session_new {
            let request: NewSessionRequest = serde_json::from_value(params)?;
            let session_id = format!("echo-session-{}", self.sessions.len() + 1);
            self.sessions.insert(session_id.clone(), request.cwd);
            return respond(output, id, &NewSessionResponse::new(session_id)).await;
        }
        if method == AGENT_METHOD_NAMES.session_close {
            let _request: CloseSessionRequest = serde_json::from_value(params)?;
            return respond(output, id, &CloseSessionResponse::new()).await;
        }
        if method == AGENT_METHOD_NAMES.session_prompt {
            let request: PromptRequest = serde_json::from_value(params)?;
            return self.prompt(output, id, request).await;
        }
        let error = json!({"code": METHOD_NOT_FOUND_CODE, "message": method});
        acp(
            output,
            json!({"jsonrpc": JSON_RPC_VERSION, "id": id, "error": error}),
        )
        .await
    }

    /// Answers one prompt according to the markers its text carries.
    async fn prompt(
        &mut self,
        output: &mut Stdout,
        id: Value,
        request: PromptRequest,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let session_id = request.session_id.to_string();
        let text = request
            .prompt
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let cwd = self.sessions.get(&session_id).cloned().unwrap_or_default();
        if text.contains("[hold]") {
            message(output, &session_id, "holding").await?;
            self.held = Some(PendingPrompt {
                request_id: id,
                session_id,
            });
            return Ok(());
        }
        if text.contains("[commit-direct]") {
            let status = std::process::Command::new("git")
                .args(["commit", "--allow-empty", "-m", "echo direct commit"])
                .current_dir(&cwd)
                .status()?;
            message(
                output,
                &session_id,
                &format!("committed directly: {status}"),
            )
            .await?;
            return respond(output, id, &PromptResponse::new(StopReason::EndTurn)).await;
        }
        if text.contains("[commit]") {
            let spawn = json!({
                "jsonrpc": JSON_RPC_VERSION,
                "id": SPAWN_REQUEST_ID,
                "method": CHILDPROCESS_SPAWN_METHOD,
                "params": {
                    "command": "git",
                    "args": ["commit", "--allow-empty", "-m", "echo host commit"],
                    "cwd": cwd,
                },
            });
            write_message(output, &spawn).await?;
            self.committing = Some(PendingPrompt {
                request_id: id,
                session_id,
            });
            return Ok(());
        }
        let answer = if text.contains("[env]") {
            let cwd = std::env::current_dir().unwrap_or_default();
            let home = std::env::var("HOME").unwrap_or_default();
            format!("cwd={} home={home}", cwd.display())
        } else if text.contains("[large]") {
            "x".repeat(LARGE_ANSWER_BYTES)
        } else {
            format!("echo: {text}")
        };
        message(output, &session_id, &answer).await?;
        respond(output, id, &PromptResponse::new(StopReason::EndTurn)).await
    }

    /// Completes the `[commit]` prompt once its host-spawned process is done.
    async fn commit_finished(
        &mut self,
        output: &mut Stdout,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(pending) = self.committing.take() {
            message(output, &pending.session_id, "committed through the host").await?;
            respond(
                output,
                pending.request_id,
                &PromptResponse::new(StopReason::EndTurn),
            )
            .await?;
        }
        Ok(())
    }
}

/// Streams one agent message chunk to the session.
async fn message(
    output: &mut Stdout,
    session_id: &str,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let notification = SessionNotification::new(
        session_id.to_string(),
        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(TextContent::new(
            text,
        )))),
    );
    acp(
        output,
        json!({
            "jsonrpc": JSON_RPC_VERSION,
            "method": CLIENT_METHOD_NAMES.session_update,
            "params": notification,
        }),
    )
    .await
}

/// Answers one ACP request.
async fn respond(
    output: &mut Stdout,
    id: Value,
    result: &impl Serialize,
) -> Result<(), Box<dyn std::error::Error>> {
    acp(
        output,
        json!({"jsonrpc": JSON_RPC_VERSION, "id": id, "result": result}),
    )
    .await
}

/// Sends one ACP frame to the host inside `agent/acp`.
async fn acp(output: &mut Stdout, frame: Value) -> Result<(), Box<dyn std::error::Error>> {
    write_message(
        output,
        &json!({"jsonrpc": JSON_RPC_VERSION, "method": AGENT_ACP_METHOD, "params": frame}),
    )
    .await?;
    Ok(())
}

/// Appends this process id to the journal in the package root.
fn record_pid() {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("echo-agent.pids")
    {
        let _ = writeln!(file, "{}", std::process::id());
    }
}
