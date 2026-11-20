//! Plugin protocol shell and the minimal ACP agent inside it.
//!
//! Given a model context, the fixture acts as a minimal adapter: it confirms the exact frozen
//! configuration it received, unless its Settings ask it to refuse (`{"refuse": true}`), so a
//! test can show that an unconfirmed adapter never reaches ACP.

use agent_client_protocol_schema::v1::{
    AGENT_METHOD_NAMES, AgentCapabilities, CLIENT_METHOD_NAMES, CancelNotification,
    CloseSessionRequest, CloseSessionResponse, ContentBlock, ContentChunk, Implementation,
    InitializeRequest, InitializeResponse, McpCapabilities, NewSessionRequest, NewSessionResponse,
    PromptRequest, PromptResponse, SessionCapabilities, SessionCloseCapabilities,
    SessionNotification, SessionUpdate, StopReason, TextContent, ToolCallStatus,
};
use ora_plugin_protocol::{
    AGENT_ACP_METHOD, AGENT_LIST_MODELS_METHOD, AGENT_START_METHOD, AGENT_STOP_METHOD,
    AgentListModelsResult, AgentModelApplied, AgentModelContext, AgentStartContext,
    AgentStartResult, CHILDPROCESS_EXIT_METHOD, CHILDPROCESS_SPAWN_METHOD, JSON_RPC_VERSION,
    METHOD_NOT_FOUND_CODE, PluginRegistrationParams, REGISTER_METHOD, SHUTDOWN_METHOD,
    read_message, write_message,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use crate::mcp_checks::{record_host_mcp, verify_mcp_frame};
use tokio::io::{Stdout, stdin, stdout};

/// Identity of the one host request this fixture sends.
const SPAWN_REQUEST_ID: &str = "echo-spawn";

/// Size of the `[large]` answer, above the 256 KiB a Thread record may hold.
const LARGE_ANSWER_BYTES: usize = 300 * 1024;

/// A prompt whose response is deferred until something else happens.
struct PendingPrompt {
    request_id: Value,
    session_id: String,
    /// What the fixture sends right after it answers a cancel of this prompt.
    after_cancel: AfterCancel,
    /// Answers a cancel with `end_turn`, as an Agent that finished first would.
    finishing: bool,
}

/// Traffic a held prompt sends after it answered a cancel.
#[derive(Clone, Copy, Default)]
enum AfterCancel {
    #[default]
    Nothing,
    /// One more message chunk: turn traffic after the response fence.
    Message,
    /// A session title: a session-level notification, which belongs to no turn.
    Title,
    /// Some providers discard their session while returning an error to the cancelled prompt.
    SessionClosed,
}

/// Everything the fixture remembers between frames.
#[derive(Default)]
struct EchoAgent {
    sessions: BTreeMap<String, PathBuf>,
    /// A safe confirmation of the session/new fixture assertion, never its setting values.
    mcp_verified: bool,
    /// A `[hold]` prompt, answered when the host cancels it.
    held: Option<PendingPrompt>,
    /// A `[commit]` prompt, answered when the host-spawned commit exits.
    committing: Option<PendingPrompt>,
    /// The model context the host started this plugin with, if any.
    model: Option<AgentModelContext>,
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
            (Some(SHUTDOWN_METHOD), _) => {
                if std::path::Path::new("hold-shutdown").exists() {
                    std::fs::write("received-shutdown", b"")?;
                    std::future::pending::<()>().await;
                }
                break;
            }
            (Some(AGENT_ACP_METHOD), None) => agent.acp_frame(&mut output, params).await?,
            (Some(CHILDPROCESS_EXIT_METHOD), None) => agent.commit_finished(&mut output).await?,
            (Some(method), Some(id)) => {
                let result = match method {
                    AGENT_START_METHOD => serde_json::to_value(agent.start(params)?)?,
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
    /// Confirms a supplied model context exactly, the way a real adapter does once it applied it.
    fn start(&mut self, params: Value) -> Result<AgentStartResult, Box<dyn std::error::Error>> {
        let start: AgentStartContext = serde_json::from_value(params)?;
        let mut result = AgentStartResult::acp_v1();
        if let Some(context) = start.model_context {
            let settings: Value = serde_json::from_str(&context.configuration.settings_json)?;
            if settings.get("refuse") != Some(&Value::Bool(true)) {
                result.model_applied = Some(AgentModelApplied {
                    adapter_version: context.configuration.adapter_version,
                    declaration_fingerprint: context.configuration.declaration_fingerprint.clone(),
                    configuration_revision: context.configuration.configuration_revision,
                });
            }
            self.model = Some(context);
        }
        Ok(result)
    }

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
                    // A `[hold-finishing]` turn finished on its own just as the cancel arrived.
                    let stop_reason = if held.finishing {
                        StopReason::EndTurn
                    } else {
                        StopReason::Cancelled
                    };
                    if matches!(held.after_cancel, AfterCancel::SessionClosed) {
                        self.sessions.remove(&held.session_id);
                        acp(output, json!({"jsonrpc":JSON_RPC_VERSION,"id":held.request_id,"error":{"code":-32603,"message":"Session is closed"}})).await?;
                    } else {
                        respond(output, held.request_id, &PromptResponse::new(stop_reason)).await?;
                    }
                    // Traffic that follows the answer is sent a little later, as an Agent's late
                    // output would be, so the host has settled the answer before it arrives.
                    if !matches!(
                        held.after_cancel,
                        AfterCancel::Nothing | AfterCancel::SessionClosed
                    ) {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    match held.after_cancel {
                        AfterCancel::Nothing | AfterCancel::SessionClosed => {}
                        AfterCancel::Message => {
                            message(output, &held.session_id, "still talking").await?;
                        }
                        AfterCancel::Title => {
                            update(
                                output,
                                &held.session_id,
                                SessionUpdate::SessionInfoUpdate(
                                    agent_client_protocol_schema::v1::SessionInfoUpdate::new()
                                        .title("Echo title"),
                                ),
                            )
                            .await?;
                        }
                    }
                }
            }
            return Ok(());
        };
        if method == AGENT_METHOD_NAMES.initialize {
            if std::path::Path::new("fail-initialize").exists() {
                return acp(output, json!({"jsonrpc":JSON_RPC_VERSION,"id":id,"error":{"code":-32603,"message":"fixture initialize failed"}})).await;
            }
            if std::path::Path::new("require-path").exists() && std::env::var_os("PATH").is_none() {
                return Err("replacement lost ambient PATH".into());
            }
            let request: InitializeRequest = serde_json::from_value(params)?;
            record_acp_method("initialize");
            let flags: Value = std::fs::read("mcp-capabilities.json")
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or(Value::Null);
            let capabilities = AgentCapabilities::new()
                .load_session(flags["load"].as_bool().unwrap_or(false))
                .mcp_capabilities(
                    McpCapabilities::new().http(flags["http"].as_bool().unwrap_or(false)),
                )
                .session_capabilities(
                    SessionCapabilities::new().close(SessionCloseCapabilities::new()),
                );
            let response = InitializeResponse::new(request.protocol_version)
                .agent_capabilities(capabilities)
                .agent_info(Implementation::new("ora-node-echo-agent", "1.0.0"));
            return respond(output, id, &response).await;
        }
        if method == AGENT_METHOD_NAMES.session_new {
            record_acp_method("session/new");
            if std::path::Path::new("fail-new").exists() {
                return acp(output, json!({"jsonrpc":JSON_RPC_VERSION,"id":id,"error":{"code":-32603,"message":"provider unavailable"}})).await;
            }
            self.mcp_verified = verify_mcp_frame(&params);
            record_host_mcp(AGENT_METHOD_NAMES.session_new, &params);
            if std::path::Path::new("stall-new").exists() {
                // Never answers, so the Node's setup window alone decides when the create fails.
                return Ok(());
            }
            let request: NewSessionRequest = serde_json::from_value(params)?;
            let session_id = format!("echo-session-{}", self.sessions.len() + 1);
            if std::fs::read("mcp-capabilities.json")
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some_and(|flags| flags["load"] == true)
            {
                std::fs::write(request.cwd.join(".echo-provider-session"), &session_id)?;
            }
            self.sessions.insert(session_id.clone(), request.cwd);
            return respond(output, id, &NewSessionResponse::new(session_id)).await;
        }
        if method == AGENT_METHOD_NAMES.session_load {
            record_acp_method("session/load");
            record_host_mcp(AGENT_METHOD_NAMES.session_load, &params);
            if std::path::Path::new("fail-load").exists() {
                return acp(output, json!({"jsonrpc":JSON_RPC_VERSION,"id":id,"error":{"code":-32602,"message":"unknown provider session"}})).await;
            }
            if let Some(delay) = std::fs::read_to_string("load-delay-ms")
                .ok()
                .and_then(|value| value.trim().parse::<u64>().ok())
            {
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
            let request: agent_client_protocol_schema::v1::LoadSessionRequest =
                serde_json::from_value(params)?;
            let session_id = request.session_id.to_string();
            if std::fs::read_to_string(request.cwd.join(".echo-provider-session"))
                .ok()
                .as_deref()
                != Some(&session_id)
            {
                return acp(output,json!({"jsonrpc": JSON_RPC_VERSION,"id":id,"error":{"code":-32602,"message":"unknown provider session"}})).await;
            }
            self.sessions.insert(session_id, request.cwd);
            return respond(
                output,
                id,
                &agent_client_protocol_schema::v1::LoadSessionResponse::new(),
            )
            .await;
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
        record_acp_method("session/prompt");
        if !self.sessions.contains_key(&session_id) {
            return acp(output, json!({"jsonrpc":JSON_RPC_VERSION,"id":id,"error":{"code":-32602,"message":"unknown provider session"}})).await;
        }
        if let Ok(delay) = std::fs::read_to_string("prompt-delay-ms") {
            std::fs::remove_file("prompt-delay-ms")?;
            let until = tokio::time::Instant::now()
                + std::time::Duration::from_millis(delay.trim().parse()?);
            while tokio::time::Instant::now() < until {
                acp(output, json!({"jsonrpc":JSON_RPC_VERSION,"method":"session/update","params":{"sessionId":session_id,"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"working "}}}})).await?;
                tokio::time::sleep_until(
                    until.min(tokio::time::Instant::now() + std::time::Duration::from_secs(10)),
                )
                .await;
            }
        }
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
        if text.contains("[session-closed]") {
            self.sessions.remove(&session_id);
            return acp(output, json!({"jsonrpc":JSON_RPC_VERSION,"id":id,"error":{"code":-32603,"message":"Session is closed"}})).await;
        }
        if text.contains("[mcp]") {
            message(
                output,
                &session_id,
                if self.mcp_verified {
                    "MCP frame verified"
                } else {
                    "MCP frame invalid"
                },
            )
            .await?;
            return respond(output, id, &PromptResponse::new(StopReason::EndTurn)).await;
        }
        if text.contains("[authentication-required]") {
            return acp(
                output,
                json!({"jsonrpc": JSON_RPC_VERSION, "id": id,
                "error": {"code": -32603, "message": "Authentication required"}}),
            )
            .await;
        }
        if text.contains("[request-failed]") {
            return acp(
                output,
                json!({"jsonrpc": JSON_RPC_VERSION, "id": id,
                "error": {"code": -32603, "message": "upstream diagnostic must not escape"}}),
            )
            .await;
        }
        if text.contains("[ignore-cancel]") {
            // Never answers, not even a cancel: the host's cancel deadline must end the session.
            message(output, &session_id, "ignoring").await?;
            return Ok(());
        }
        if text.contains("[leak-running]") {
            // Names the `[hold-done-tool]` turn's tool call with a status that keeps the record
            // pending, so it would reach only the live preview, then holds.
            tool(output, &session_id, "echo-tool", ToolCallStatus::InProgress).await?;
            self.held = Some(PendingPrompt {
                request_id: id,
                session_id,
                after_cancel: AfterCancel::Nothing,
                finishing: false,
            });
            return Ok(());
        }
        if text.contains("[leak]") {
            // Names the tool call a `[hold-done-tool]` turn ran, as if that turn still produced.
            tool(output, &session_id, "echo-tool", ToolCallStatus::Completed).await?;
            return respond(output, id, &PromptResponse::new(StopReason::EndTurn)).await;
        }
        if text.contains("[hold") {
            message(output, &session_id, "holding").await?;
            if text.contains("[hold-many]") {
                for number in 0..300 {
                    tool(
                        output,
                        &session_id,
                        &format!("tool-{number}"),
                        ToolCallStatus::Completed,
                    )
                    .await?;
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            }
            if text.contains("[hold-running-tool]") {
                tool(output, &session_id, "echo-tool", ToolCallStatus::InProgress).await?;
            }
            if text.contains("[hold-done-tool]") {
                tool(output, &session_id, "echo-tool", ToolCallStatus::Completed).await?;
            }
            let after_cancel = if text.contains("[hold-chatty]") {
                AfterCancel::Message
            } else if text.contains("[hold-titled]") {
                AfterCancel::Title
            } else if text.contains("[hold-close]") {
                AfterCancel::SessionClosed
            } else {
                AfterCancel::Nothing
            };
            self.held = Some(PendingPrompt {
                request_id: id,
                session_id,
                after_cancel,
                finishing: text.contains("[hold-finishing]"),
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
        if text.contains("[commit-host]") {
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
                after_cancel: AfterCancel::Nothing,
                finishing: false,
            });
            return Ok(());
        }
        if text.contains("[write]")
            || text.contains("[commit]")
            || text.contains("[push]")
            || text.contains("[fetch ")
        {
            let answer = git_markers(&cwd, &text)?;
            message(output, &session_id, &answer).await?;
            return respond(output, id, &PromptResponse::new(StopReason::EndTurn)).await;
        }
        let answer = if text.contains("[model]") {
            // Reports what the adapter received, whether the token also leaked into the
            // environment every process the plugin spawns would inherit, and whether the Node's
            // own environment (here the test runner's `CARGO*` variables) reached it at all.
            match &self.model {
                Some(model) => format!(
                    "model={} proxy={} token_in_environment={} host_environment={}",
                    model.configuration.model.id,
                    model.proxy_base_url,
                    std::env::vars().any(|(_, value)| value.contains(&model.token)),
                    std::env::vars().any(|(name, _)| name.starts_with("CARGO")),
                ),
                None => "model=none".to_string(),
            }
        } else if text.contains("[env]") {
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

/// Streams one session update.
async fn update(
    output: &mut Stdout,
    session_id: &str,
    update: SessionUpdate,
) -> Result<(), Box<dyn std::error::Error>> {
    let notification = SessionNotification::new(session_id.to_string(), update);
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

/// Reports one tool call with the given status.
async fn tool(
    output: &mut Stdout,
    session_id: &str,
    tool_call_id: &str,
    status: ToolCallStatus,
) -> Result<(), Box<dyn std::error::Error>> {
    update(
        output,
        session_id,
        SessionUpdate::ToolCall(
            agent_client_protocol_schema::v1::ToolCall::new(tool_call_id.to_owned(), "Echo tool")
                .status(status),
        ),
    )
    .await
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
    // Version discovery may invoke the Deno stand-in outside a package; never litter that cwd.
    if !std::path::Path::new("orax.toml").is_file() {
        return;
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("echo-agent.pids")
    {
        // One write call: sibling sessions append while each other's process starts, and a
        // split write would tear the line the tests read back.
        let _ = file.write_all(format!("{}\n", std::process::id()).as_bytes());
    }
}

/// Executes the collaboration markers in task order, excluding the contextual material the host
/// appends. Commands use argument arrays so task text never becomes shell code.
fn git_markers(cwd: &std::path::Path, prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    let task = prompt.split("\n\nContext\n").next().unwrap_or(prompt);
    let mut rest = task;
    let mut results = Vec::new();
    while let Some(open) = rest.find('[') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find(']') else { break };
        let marker = &rest[..close];
        rest = &rest[close + 1..];
        let result = match marker {
            "write" => {
                std::fs::write(cwd.join("echo.txt"), format!("{task}\n"))?;
                continue;
            }
            "commit" => {
                let added = git_command(cwd, &["add", "-A"])?;
                if !added.status.success() {
                    added
                } else {
                    git_command(
                        cwd,
                        &[
                            "-c",
                            "user.name=echo",
                            "-c",
                            "user.email=echo@ora.invalid",
                            "commit",
                            "-m",
                            &format!("echo: {task}"),
                        ],
                    )?
                }
            }
            "push" => ora_git_command(cwd, &["push"])?,
            _ => {
                let Some(workspace) = marker.strip_prefix("fetch ") else {
                    continue;
                };
                let fetched = ora_git_command(cwd, &["fetch", "workspace", workspace])?;
                if !fetched.status.success() {
                    fetched
                } else {
                    git_command(
                        cwd,
                        &[
                            "-c",
                            "user.name=echo",
                            "-c",
                            "user.email=echo@ora.invalid",
                            "merge",
                            "--no-edit",
                            &format!("refs/remotes/ora/workspaces/{workspace}/main"),
                        ],
                    )?
                }
            }
        };
        let code = if result.status.success() {
            "ok".to_owned()
        } else {
            String::from_utf8_lossy(&result.stderr)
                .lines()
                .find_map(|line| {
                    line.strip_prefix("ora-git: ")
                        .and_then(|s| s.split(':').next())
                })
                .unwrap_or("git_failed")
                .to_owned()
        };
        results.push(format!("[{marker}: {code}]"));
    }
    Ok(format!("echo: {task} {}", results.join(" ")))
}

/// Runs fixture Git without emitting diagnostics on the plugin protocol stream.
fn git_command(cwd: &std::path::Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    // The session's Git identity reaches the agent as `GIT_AUTHOR_*`/`GIT_COMMITTER_*`, which
    // would win over `user.*`; the markers commit as the fixed echo identity instead.
    std::process::Command::new("git")
        .current_dir(cwd)
        .args(args)
        .env("GIT_AUTHOR_NAME", "echo")
        .env("GIT_AUTHOR_EMAIL", "echo@ora.invalid")
        .env("GIT_COMMITTER_NAME", "echo")
        .env("GIT_COMMITTER_EMAIL", "echo@ora.invalid")
        .output()
}

/// Runs the `ora-git` CLI through this fixture's own executable, which is always built with the
/// tests; the session socket in the environment is inherited unchanged.
fn ora_git_command(cwd: &std::path::Path, args: &[&str]) -> std::io::Result<std::process::Output> {
    std::process::Command::new(std::env::current_exe()?)
        .current_dir(cwd)
        .arg("ora-git")
        .args(args)
        .output()
}

/// Records only method names so failure tests can prove no session/new was sent.
fn record_acp_method(method: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("echo-acp-methods")
    {
        let _ = writeln!(file, "{method}");
    }
}
