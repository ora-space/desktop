//! On-demand AI diagnosis of a failed agent node. The result is stored as `payload.ai_diagnosis`
//! and is never consulted by scheduling, resume, or rollback.

use super::executor::{AssistantOutputAccumulator, resolve_agent_ref};
use crate::agent_runtime::AgentRuntimeManager;
use crate::error::BackendError;
use crate::session_setup::SessionMcpSelection;
use agent_client_protocol_schema::v1::{ContentBlock, SessionUpdate, TextContent};
use ora_application::{
    AgentExecutor, AgentOutputContract, ApplicationError, NodeFailureDetail, NodeType,
    WorkflowGraph, WorkflowRunEngineRepository, WorkflowRunPayload,
};
use ora_contracts::{
    PromptSessionEvent, PromptSessionRequest, StartSessionRequest, StopSessionRequest,
    WorkflowRunLocale,
};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_domain::{SessionId, WorkflowNodeRunId, WorkflowNodeStatus, WorkflowRunId, WorkspaceId};
use ora_history::{HistoryRecord, SessionHistory, read_session_history};
use ora_logging::ora_warn;
use std::collections::BTreeSet;
use std::path::Path;
use thiserror::Error;

const CONVERSATION_MESSAGE_CHAR_LIMIT: usize = 1500;
const LAST_OUTPUT_CHAR_LIMIT: usize = 3000;
const MAX_PROMPT_CHARS: usize = 20_000;
const CONVERSATION_TAIL_RECORDS: usize = 8;

/// Everything the diagnosis prompt is built from; assembled from the DB and the session history.
#[derive(Debug)]
pub(super) struct DiagnosisInput {
    pub locale: WorkflowRunLocale,
    pub node_title: String,
    pub node_description: String,
    pub node_prompt: String,
    pub output_schema: Option<serde_json::Value>,
    pub error: DiagnosisError,
    pub last_output: Option<String>,
    pub conversation_tail: Vec<(String, String)>,
    pub changed_paths: Vec<String>,
}

/// Failure facts shown to the diagnosis model; `kind` is a string so a missing payload can be `"unknown"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DiagnosisError {
    pub kind: String,
    pub message: String,
    pub source_chain: Vec<String>,
    pub attempt: u32,
}

#[derive(Debug, Error)]
#[error("{0}")]
struct DiagnosisFailure(&'static str);

/// Loads the failed agent node's context. Missing history is an empty tail, not an error.
pub(super) fn load_input(
    pool: &RepositoryPool,
    sessions_root: &Path,
    run_id: &str,
    node_id: &str,
) -> Result<
    (
        DiagnosisInput,
        AgentExecutor,
        WorkspaceId,
        WorkflowNodeRunId,
    ),
    BackendError,
> {
    let run_id = WorkflowRunId::new(run_id);
    let repository = SqliteWorkflowRunEngineRepository::new(pool.clone());
    let context = repository
        .find_execution_context(&run_id)
        .map_err(repository_error)?
        .ok_or_else(|| ApplicationError::WorkflowRunNotFound {
            run_id: run_id.to_string(),
        })?;
    let node_run = repository
        .list_node_runs(&run_id)
        .map_err(repository_error)?
        .into_iter()
        .find(|node_run| node_run.node_id == node_id)
        .ok_or(ApplicationError::WorkflowNodeNotDiagnosable)?;
    if node_run.status != WorkflowNodeStatus::Failed {
        return Err(ApplicationError::WorkflowNodeNotDiagnosable.into());
    }
    let graph = WorkflowGraph::parse(&context.graph_json)
        .map_err(ApplicationError::WorkflowRunGraphParse)?;
    let node = graph
        .node(&node_run.node_id)
        .ok_or(ApplicationError::WorkflowNodeNotDiagnosable)?;
    if node.node_type != NodeType::Agent {
        return Err(ApplicationError::WorkflowNodeNotDiagnosable.into());
    }
    let config = node
        .agent_config
        .as_ref()
        .ok_or(ApplicationError::WorkflowNodeNotDiagnosable)?;
    let locale = context
        .run
        .payload
        .as_deref()
        .and_then(|payload| serde_json::from_str::<WorkflowRunPayload>(payload).ok())
        .map(|payload| payload.locale)
        .unwrap_or(WorkflowRunLocale::ZhCn);
    let output_schema = match &config.output_contract {
        Some(AgentOutputContract::Structured { schema, .. }) => Some(schema.clone()),
        Some(AgentOutputContract::None) | Some(AgentOutputContract::Text) | None => None,
    };
    let (error, changed_paths) =
        parse_payload(node_run.payload.as_deref(), node_run.error.as_deref());
    let conversation_tail = match node_run.session_id.as_ref() {
        Some(session_id) => match read_session_history(sessions_root, session_id.as_ref()) {
            Ok(history) => conversation_tail_from_history(&history),
            Err(_) => Vec::new(),
        },
        None => Vec::new(),
    };
    let last_output = node_run
        .output
        .as_deref()
        .map(|output| truncate_chars(output, LAST_OUTPUT_CHAR_LIMIT));
    let input = DiagnosisInput {
        locale,
        node_title: node.title.clone(),
        node_description: node.description.clone(),
        node_prompt: config.prompt.clone(),
        output_schema,
        error,
        last_output,
        conversation_tail,
        changed_paths,
    };
    Ok((
        input,
        config.executor.clone(),
        context.workspace.id,
        node_run.id,
    ))
}

/// Builds the locale-specific diagnosis prompt and caps it at [`MAX_PROMPT_CHARS`].
///
/// Conversation tail is truncated first so the closing "please answer" section stays intact.
pub(super) fn build_diagnosis_prompt(input: &DiagnosisInput) -> String {
    let mut conversation = input.conversation_tail.clone();
    let mut last_output = input.last_output.clone();
    loop {
        let prompt = render_prompt(input, &conversation, last_output.as_deref());
        let len = prompt.chars().count();
        if len <= MAX_PROMPT_CHARS {
            return prompt;
        }
        let excess = len - MAX_PROMPT_CHARS;
        if !conversation.is_empty() {
            shrink_oldest_conversation(&mut conversation, excess);
            continue;
        }
        if let Some(output) = last_output.as_mut() {
            let keep = output.chars().count().saturating_sub(excess.max(1));
            if keep == 0 {
                last_output = None;
            } else {
                *output = output.chars().take(keep).collect();
            }
            continue;
        }
        return prompt.chars().take(MAX_PROMPT_CHARS).collect();
    }
}

/// Starts an unpublished diagnosis session with no MCPs, prompts it, then always stops and discards it.
pub(super) async fn run_diagnosis(
    agent_runtime: &AgentRuntimeManager,
    workspace_id: WorkspaceId,
    executor: &AgentExecutor,
    prompt: String,
) -> Result<String, BackendError> {
    let agent_ref = resolve_agent_ref(&executor.agent_cli)
        .map_err(|error| BackendError::internal("diagnosis agent ref is invalid", error))?;
    let started = agent_runtime
        .start_workflow_node_session(
            StartSessionRequest {
                workspace_id: workspace_id.to_string(),
                agent_ref,
                model: Some(executor.model_id.clone()),
            },
            SessionMcpSelection::Explicit(BTreeSet::new()),
        )
        .await?;
    let session_id = SessionId::new(started.session.id);
    let result = collect_diagnosis_text(agent_runtime, &session_id, prompt).await;
    match agent_runtime
        .stop_session(StopSessionRequest {
            session_id: session_id.to_string(),
        })
        .await
    {
        Ok(_) => {}
        Err(error) => {
            ora_warn!(
                session_id = %session_id,
                error = %error,
                "failed to stop diagnosis session"
            );
        }
    }
    match agent_runtime
        .discard_unpublished_workflow_node_session(&session_id)
        .await
    {
        Ok(()) => {}
        Err(error) => {
            ora_warn!(
                session_id = %session_id,
                error = %error,
                "failed to discard unpublished diagnosis session"
            );
        }
    }
    result
}

/// Prompts the diagnosis session and keeps only the final assistant text.
async fn collect_diagnosis_text(
    agent_runtime: &AgentRuntimeManager,
    session_id: &SessionId,
    prompt: String,
) -> Result<String, BackendError> {
    let mut stream = agent_runtime
        .prompt_session(PromptSessionRequest {
            session_id: session_id.to_string(),
            prompt: vec![ContentBlock::Text(TextContent::new(prompt))],
            record_prompt: None,
            model: None,
        })
        .await?;
    let mut accumulator = AssistantOutputAccumulator::default();
    let mut completed = false;
    while let Some(event) = stream.recv().await {
        match event? {
            PromptSessionEvent::SessionUpdate { update, .. } => {
                accumulator.consume(&update);
            }
            PromptSessionEvent::PermissionRequest(_) => {}
            PromptSessionEvent::Retrying { .. } => {}
            PromptSessionEvent::Completed { .. } => {
                completed = true;
                break;
            }
        }
    }
    if !completed {
        return Err(BackendError::internal(
            "diagnosis session ended without a stop reason",
            DiagnosisFailure("diagnosis session ended without a stop reason"),
        ));
    }
    accumulator.into_output().ok_or_else(|| {
        BackendError::internal(
            "diagnosis produced no text",
            DiagnosisFailure("diagnosis produced no text"),
        )
    })
}

fn parse_payload(payload: Option<&str>, node_error: Option<&str>) -> (DiagnosisError, Vec<String>) {
    let parsed = payload.and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
    let error = parsed
        .as_ref()
        .and_then(|value| value.get("error_detail"))
        .cloned()
        .and_then(|value| serde_json::from_value::<NodeFailureDetail>(value).ok())
        .map(|detail| DiagnosisError {
            kind: detail.kind.as_str().to_string(),
            message: detail.message,
            source_chain: detail.source_chain,
            attempt: detail.attempt,
        })
        .unwrap_or_else(|| DiagnosisError {
            kind: "unknown".to_string(),
            message: node_error.unwrap_or_default().to_string(),
            source_chain: Vec::new(),
            attempt: 1,
        });
    let changed_paths = parsed
        .as_ref()
        .and_then(|value| value.get("file_changes"))
        .and_then(|value| value.as_array())
        .map(|changes| {
            changes
                .iter()
                .filter_map(|change| {
                    change
                        .get("path")
                        .and_then(|path| path.as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    (error, changed_paths)
}

fn conversation_tail_from_history(history: &SessionHistory) -> Vec<(String, String)> {
    let mut lines = Vec::new();
    for line in &history.lines {
        let HistoryRecord::Update { update, .. } = &line.record else {
            continue;
        };
        if let Some(entry) = conversation_line(update) {
            lines.push(entry);
        }
    }
    let start = lines.len().saturating_sub(CONVERSATION_TAIL_RECORDS);
    lines.split_off(start)
}

fn conversation_line(update: &SessionUpdate) -> Option<(String, String)> {
    let (role, content) = match update {
        SessionUpdate::AgentMessageChunk(chunk) => ("assistant", &chunk.content),
        SessionUpdate::UserMessageChunk(chunk) => ("user", &chunk.content),
        _ => return None,
    };
    let ContentBlock::Text(text) = content else {
        return None;
    };
    if text.text.is_empty() {
        return None;
    }
    Some((
        role.to_string(),
        truncate_chars(&text.text, CONVERSATION_MESSAGE_CHAR_LIMIT),
    ))
}

fn render_prompt(
    input: &DiagnosisInput,
    conversation: &[(String, String)],
    last_output: Option<&str>,
) -> String {
    let copy = prompt_copy(input.locale);
    let mut sections = Vec::new();
    sections.push(copy.role.to_string());
    sections.push(format!(
        "{}\n{}{}\n{}{}\n{}\n```\n{}\n```",
        copy.failed_step,
        copy.title,
        input.node_title,
        copy.description,
        input.node_description,
        copy.node_prompt,
        input.node_prompt
    ));
    if let Some(schema) = &input.output_schema {
        let schema = serde_json::to_string_pretty(schema).unwrap_or_else(|_| schema.to_string());
        sections.push(format!(
            "{}\n```json\n{schema}\n```",
            copy.output_requirements
        ));
    }
    let mut error = format!(
        "{}\nkind: {}\nmessage: {}\nattempt: {}",
        copy.error, input.error.kind, input.error.message, input.error.attempt
    );
    if !input.error.source_chain.is_empty() {
        error.push('\n');
        error.push_str(copy.source_chain);
        for line in &input.error.source_chain {
            error.push_str("\n- ");
            error.push_str(line);
        }
    }
    sections.push(error);
    if let Some(output) = last_output {
        sections.push(format!("{}\n```\n{output}\n```", copy.last_output));
    }
    let mut tail = copy.conversation_tail.to_string();
    for (role, text) in conversation {
        tail.push_str("\n- ");
        tail.push_str(role);
        tail.push_str(": ");
        tail.push_str(text);
    }
    sections.push(tail);
    let mut files = copy.changed_files.to_string();
    for path in &input.changed_paths {
        files.push_str("\n- ");
        files.push_str(path);
    }
    sections.push(files);
    sections.push(format!(
        "{}\n{}",
        copy.please_answer, copy.answer_instructions
    ));
    sections.join("\n\n")
}

struct PromptCopy {
    role: &'static str,
    failed_step: &'static str,
    title: &'static str,
    description: &'static str,
    node_prompt: &'static str,
    output_requirements: &'static str,
    error: &'static str,
    source_chain: &'static str,
    last_output: &'static str,
    conversation_tail: &'static str,
    changed_files: &'static str,
    please_answer: &'static str,
    answer_instructions: &'static str,
}

fn prompt_copy(locale: WorkflowRunLocale) -> PromptCopy {
    match locale {
        WorkflowRunLocale::ZhCn => PromptCopy {
            role: "你是工作流故障诊断助手。根据下面的失败上下文分析原因并给出修复建议。只输出纯文本，不要调用任何工具，不要修改文件。",
            failed_step: "## 失败的步骤",
            title: "标题：",
            description: "描述：",
            node_prompt: "提示词：",
            output_requirements: "## 输出要求",
            error: "## 错误信息",
            source_chain: "source:",
            last_output: "## 上次最终输出",
            conversation_tail: "## 会话末尾",
            changed_files: "## 改动的文件",
            please_answer: "## 请回答",
            answer_instructions: "1. 用两三句话说清楚发生了什么；2. 最可能的根因（最多三条，按可能性排序）；3. 具体修复建议（改提示词/改配置/改代码/换模型，各一句）。控制在 300 字以内。",
        },
        WorkflowRunLocale::EnUs => PromptCopy {
            role: "You are a workflow failure diagnosis assistant. Analyze the failure below and suggest fixes. Output plain text only; do not call any tools or modify files.",
            failed_step: "## Failed step",
            title: "Title: ",
            description: "Description: ",
            node_prompt: "Prompt:",
            output_requirements: "## Output requirements",
            error: "## Error",
            source_chain: "source:",
            last_output: "## Last final output",
            conversation_tail: "## End of conversation",
            changed_files: "## Changed files",
            please_answer: "## Please answer",
            answer_instructions: "1. In two or three sentences, say what happened; 2. The most likely root causes (at most three, ordered by likelihood); 3. Concrete fix suggestions (prompt / config / code / model, one sentence each). Keep the answer under 300 characters.",
        },
    }
}

fn shrink_oldest_conversation(conversation: &mut Vec<(String, String)>, excess: usize) {
    let Some((_, text)) = conversation.first_mut() else {
        return;
    };
    let keep = text.chars().count().saturating_sub(excess.max(1));
    if keep == 0 {
        conversation.remove(0);
    } else {
        *text = text.chars().take(keep).collect();
    }
}

fn truncate_chars(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn repository_error(source: ora_application::RepositoryError) -> BackendError {
    BackendError::from(ApplicationError::WorkflowRunRepository { source })
}

#[cfg(test)]
mod tests {
    use super::{
        DiagnosisError, DiagnosisInput, MAX_PROMPT_CHARS, build_diagnosis_prompt, load_input,
    };
    use crate::error::BackendError;
    use crate::workflow::run::test_fixture::{
        AGENT_GRAPH, NoopExecutor, bootstrap, started_run_with,
    };
    use ora_application::{NodeFailure, NodeFailureKind};
    use ora_contracts::{EmptyErrorParams, PublicError, WorkflowRunLocale};
    use pretty_assertions::assert_eq;

    fn sample_input(locale: WorkflowRunLocale) -> DiagnosisInput {
        DiagnosisInput {
            locale,
            node_title: "审查".to_string(),
            node_description: "检查补丁".to_string(),
            node_prompt: "review the patch".to_string(),
            output_schema: Some(serde_json::json!({"type":"object","required":["ok"]})),
            error: DiagnosisError {
                kind: "structured_output".to_string(),
                message: "not json".to_string(),
                source_chain: vec!["expected object".to_string()],
                attempt: 2,
            },
            last_output: Some("almost".to_string()),
            conversation_tail: vec![("assistant".to_string(), "I will try JSON".to_string())],
            changed_paths: vec!["src/main.rs".to_string()],
        }
    }

    #[test]
    fn build_diagnosis_prompt_zh_contains_every_section() {
        let prompt = build_diagnosis_prompt(&sample_input(WorkflowRunLocale::ZhCn));
        println!("ZH_PROMPT_BEGIN\n{prompt}\nZH_PROMPT_END");
        assert!(prompt.contains("## 失败的步骤"));
        assert!(prompt.contains("## 输出要求"));
        assert!(prompt.contains("## 错误信息"));
        assert!(prompt.contains("## 上次最终输出"));
        assert!(prompt.contains("## 会话末尾"));
        assert!(prompt.contains("## 改动的文件"));
        assert!(prompt.contains("## 请回答"));
        assert!(prompt.contains("```\nreview the patch\n```"));
        assert!(prompt.contains("\"type\": \"object\""));
        assert!(prompt.contains("\"ok\""));
        assert!(prompt.contains("kind: structured_output"));
        assert!(prompt.contains("message: not json"));
        assert!(prompt.contains("attempt: 2"));
        assert!(prompt.contains("- assistant: I will try JSON"));
        assert!(prompt.contains("- src/main.rs"));
    }

    #[test]
    fn build_diagnosis_prompt_en_contains_english_headers() {
        let prompt = build_diagnosis_prompt(&sample_input(WorkflowRunLocale::EnUs));
        assert!(prompt.contains("## Failed step"));
        assert!(prompt.contains("## Output requirements"));
        assert!(prompt.contains("## Error"));
        assert!(prompt.contains("## Last final output"));
        assert!(prompt.contains("## End of conversation"));
        assert!(prompt.contains("## Changed files"));
        assert!(prompt.contains("## Please answer"));
    }

    #[test]
    fn build_diagnosis_prompt_truncates_a_huge_conversation_tail() {
        let mut input = sample_input(WorkflowRunLocale::ZhCn);
        input.conversation_tail = vec![("assistant".to_string(), "x".repeat(30_000))];
        let prompt = build_diagnosis_prompt(&input);
        assert!(prompt.chars().count() <= MAX_PROMPT_CHARS);
        let answer = prompt.rfind("## 请回答").expect("answer section");
        assert!(prompt[answer..].contains("用两三句话"));
        assert_eq!(prompt[answer..].lines().next(), Some("## 请回答"));
    }

    #[test]
    fn load_input_rejects_a_succeeded_node() {
        let (temp, pool) = bootstrap();
        let (run_id, node_runs, engine) = started_run_with(&temp, &pool, AGENT_GRAPH, NoopExecutor);
        let agent = node_runs
            .iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        engine
            .complete_node(
                &run_id,
                &agent.id,
                Some("ok".to_string()),
                /*structured_output*/ None,
                /*stop_reason*/ None,
                Vec::new(),
            )
            .unwrap();
        let error = load_input(&pool, temp.path(), run_id.as_ref(), "agent").unwrap_err();
        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeNotDiagnosable(EmptyErrorParams {})
        );
        let _: BackendError = error;
    }

    #[test]
    fn load_input_rejects_a_non_agent_node() {
        let (temp, pool) = bootstrap();
        let (run_id, node_runs, engine) = started_run_with(&temp, &pool, AGENT_GRAPH, NoopExecutor);
        let agent = node_runs
            .iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        engine
            .fail_node(
                &run_id,
                &agent.id,
                NodeFailure::new(NodeFailureKind::Session, "boom"),
            )
            .unwrap();
        let start = node_runs
            .iter()
            .find(|node| node.node_id == "start")
            .expect("start node");
        let conn = rusqlite::Connection::open(temp.path().join("repository.sqlite3")).unwrap();
        conn.execute(
            "UPDATE workflow_node_runs SET status = 3 WHERE id = ?1",
            rusqlite::params![start.id.as_ref()],
        )
        .unwrap();
        let error = load_input(&pool, temp.path(), run_id.as_ref(), "start").unwrap_err();
        assert_eq!(
            error.public_error(),
            &PublicError::WorkflowNodeNotDiagnosable(EmptyErrorParams {})
        );
    }

    #[test]
    fn resume_preview_is_identical_with_and_without_ai_diagnosis() {
        use crate::workflow::run::rollback::preview;
        let (temp, pool) = bootstrap();
        let (run_id, node_runs, engine) = started_run_with(&temp, &pool, AGENT_GRAPH, NoopExecutor);
        let agent = node_runs
            .iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        engine
            .fail_node(
                &run_id,
                &agent.id,
                NodeFailure::new(NodeFailureKind::StructuredOutput, "bad json"),
            )
            .unwrap();
        let workspace = temp.path().join("fixture-project");
        std::fs::create_dir_all(&workspace).unwrap();
        let before = preview(&pool, &workspace, &run_id).unwrap();
        let conn = rusqlite::Connection::open(temp.path().join("repository.sqlite3")).unwrap();
        let current: Option<String> = conn
            .query_row(
                "SELECT payload FROM workflow_node_runs WHERE id = ?1",
                rusqlite::params![agent.id.as_ref()],
                |row| row.get(0),
            )
            .unwrap();
        let mut payload = current
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        payload["ai_diagnosis"] = serde_json::json!({
            "text": "the schema was too strict",
            "agentCli": "open_code",
            "model": "m",
            "generatedAt": 1
        });
        conn.execute(
            "UPDATE workflow_node_runs SET payload = ?2 WHERE id = ?1",
            rusqlite::params![agent.id.as_ref(), payload.to_string()],
        )
        .unwrap();
        let after = preview(&pool, &workspace, &run_id).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn load_input_accepts_a_failed_node_without_error_detail() {
        let (temp, pool) = bootstrap();
        let (run_id, node_runs, engine) = started_run_with(&temp, &pool, AGENT_GRAPH, NoopExecutor);
        let agent = node_runs
            .iter()
            .find(|node| node.node_id == "agent")
            .expect("agent node");
        engine
            .fail_node(
                &run_id,
                &agent.id,
                NodeFailure::new(NodeFailureKind::Session, "boom"),
            )
            .unwrap();
        let conn = rusqlite::Connection::open(temp.path().join("repository.sqlite3")).unwrap();
        conn.execute(
            "UPDATE workflow_node_runs SET payload = NULL WHERE id = ?1",
            rusqlite::params![agent.id.as_ref()],
        )
        .unwrap();
        let (input, executor, _workspace_id, node_run_id) =
            load_input(&pool, temp.path(), run_id.as_ref(), "agent").unwrap();
        assert_eq!(input.error.kind, "unknown");
        assert_eq!(input.error.message, "boom");
        assert_eq!(executor.agent_cli, "open_code");
        assert_eq!(executor.model_id, "m");
        assert_eq!(node_run_id, agent.id);
    }
}
