//! File-based journals written beside the fake agent binary for Desktop E2E assertions.

use agent_client_protocol_schema::v1::{McpServer, Meta};
use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

const ACP_JOURNAL: &str = "acp_calls.txt";
const PROMPT_JOURNAL: &str = "acp_prompts.jsonl";
const SETUP_JOURNAL: &str = "acp_setups.jsonl";
const COMPLETION_JOURNAL: &str = "mcp_completion_calls.jsonl";

/// Appends one served session call to the journal beside this package.
pub(super) fn record_acp_call(method: &str, session_id: &str) {
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(ACP_JOURNAL)
    {
        let _ = writeln!(file, "{method} {session_id}");
    }
}

/// Records one served prompt so tests can assert prompt-level content that never belongs in a
/// node's output (injected context blocks, required-skill invocations).
pub(super) fn record_prompt(session_id: &str, prompt: &str) {
    let result = OpenOptions::new()
        .create(true)
        .append(true)
        .open(PROMPT_JOURNAL)
        .and_then(|mut file| {
            writeln!(
                file,
                "{}",
                json!({"sessionId": session_id, "prompt": prompt})
            )
        });
    if let Err(error) = result {
        eprintln!("fake-agent could not record the prompt for session `{session_id}`: {error}");
    }
}

/// Records ACP `session/new` / `session/load` setup fields for skill root assertions.
pub(super) fn record_setup_call(
    method: &str,
    session_id: &str,
    additional_directories: &[PathBuf],
    meta: Option<&Meta>,
) {
    let entry = json!({
        "method": method,
        "sessionId": session_id,
        "additionalDirectories": additional_directories,
        "skillDirectories": meta.and_then(|map| map.get("ora/skillDirectories")),
    });
    let result = OpenOptions::new()
        .create(true)
        .append(true)
        .open(SETUP_JOURNAL)
        .and_then(|mut file| writeln!(file, "{entry}"));
    if let Err(error) = result {
        eprintln!("fake-agent could not record setup for session `{session_id}`: {error}");
    }
}

/// Records only server identities so integration assertions never journal MCP credentials.
pub(super) fn record_mcp_call(method: &str, session_id: &str, servers: &[McpServer]) {
    let names: Vec<_> = servers
        .iter()
        .map(|server| match server {
            McpServer::Stdio(server) => server.name.clone(),
            McpServer::Http(server) => server.name.clone(),
            McpServer::Sse(server) => server.name.clone(),
            _ => panic!("unsupported fixture MCP transport"),
        })
        .collect();
    let result = OpenOptions::new()
        .create(true)
        .append(true)
        .open("mcp_calls.jsonl")
        .and_then(|mut file| {
            writeln!(
                file,
                "{}",
                json!({"method": method, "sessionId": session_id, "servers": names})
            )
        });
    if let Err(error) = result {
        eprintln!(
            "fake-agent could not record MCP call `{method}` for session `{session_id}`: {error}"
        );
    }
}

/// Journals one completion tool call without the attempt token or the endpoint's server token.
pub(super) fn record_completion_call(outcome: Value) {
    let result = OpenOptions::new()
        .create(true)
        .append(true)
        .open(COMPLETION_JOURNAL)
        .and_then(|mut file| writeln!(file, "{outcome}"));
    if let Err(error) = result {
        eprintln!("fake-agent could not record the completion tool call: {error}");
    }
}
