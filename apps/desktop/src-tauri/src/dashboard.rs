//! Embedded trace dashboard wiring: resolves an Ora session to a trace file,
//! writes the dashboard locator, and returns the iframe URL Ora renders.
//!
//! The dashboard itself is an externally-managed Streamlit server the user runs
//! on a configured `dashboardHost:dashboardPort` (ADR-0001). Ora never spawns or
//! owns that process. Given the current Ora session id, this module resolves the
//! private agent session identifier + agent family + worktree cwd (backend-only,
//! ADR-0003), locates the agent-written trace file, and writes a tiny locator
//! JSON the dashboard reads by OS convention (ADR-0002). The agent session id
//! never leaves Rust: only the resolved file path is handed to the dashboard.

use crate::error::CommandError;
use crate::state::DesktopState;
use ora_contracts::AgentCli as ContractAgentCli;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;

/// The canonical dashboard agent-family name the trace visualizer renders under.
/// Mirrors the dashboard's `app_mode` dispatch; `Nga` is an `opencode` variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DashboardAgentType {
    Opencode,
    ClaudeCode,
}

impl DashboardAgentType {
    /// Returns the string Ora puts in the iframe URL and the locator JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Opencode => "opencode",
            Self::ClaudeCode => "claude_code",
        }
    }
}

/// Normalizes the persisted contract CLI selection into the dashboard agent family.
pub fn dashboard_agent_type(agent_cli: ContractAgentCli) -> DashboardAgentType {
    match agent_cli {
        ContractAgentCli::OpenCode | ContractAgentCli::Nga => DashboardAgentType::Opencode,
        ContractAgentCli::CodeAgentCli => DashboardAgentType::ClaudeCode,
    }
}

/// The JSON Ora writes to `<app_data_dir>/dashboard/<oraSessionId>.json` for the
/// dashboard to read. Carries only the resolved trace file path and agent family.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardLocator {
    pub trace_file_path: String,
    pub agent_type: String,
}

impl DashboardLocator {
    /// Returns the conventional locator directory under one Ora app-data directory.
    pub fn directory_for(app_data_directory: &Path) -> PathBuf {
        app_data_directory.join("dashboard")
    }

    /// Returns the locator file path for one Ora session id.
    pub fn file_for(app_data_directory: &Path, ora_session_id: &str) -> PathBuf {
        Self::directory_for(app_data_directory).join(format!("{ora_session_id}.json"))
    }
}

/// Writes the locator atomically so the dashboard never reads a half-written file.
pub fn write_locator(
    app_data_directory: &Path,
    ora_session_id: &str,
    locator: &DashboardLocator,
) -> Result<(), std::io::Error> {
    let directory = DashboardLocator::directory_for(app_data_directory);
    std::fs::create_dir_all(&directory)?;
    let target = DashboardLocator::file_for(app_data_directory, ora_session_id);
    // Write to a sibling temp file then rename for an atomic publish.
    let temp_directory = target
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let mut temp = tempfile::NamedTempFile::new_in(&temp_directory)?;
    use std::io::Write;
    write!(temp, "{}", serde_json::to_string_pretty(locator).map_err(std::io::Error::other)?)?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist(&target).map(|_| ()).map_err(|e| e.error)
}

/// Locates the Claude Code transcript JSONL for one agent session id.
///
/// Claude auto-saves every interactive session to
/// `~/.claude/projects/<project-hash>/<claude-session-id>.jsonl`; the project hash
/// is derived from the working directory and is not worth recomputing, so this
/// scans every project directory and matches by file name (the agent session id).
pub fn resolve_claude_code_trace(
    home_directory: &Path,
    agent_session_id: &str,
) -> Result<Option<PathBuf>, std::io::Error> {
    let projects_root = home_directory.join(".claude").join("projects");
    if !projects_root.is_dir() {
        return Ok(None);
    }
    let target_name = format!("{agent_session_id}.jsonl");
    for path in walkdir_files(&projects_root)? {
        if path
            .file_name()
            .map(|n| n == target_name.as_str())
            .unwrap_or(false)
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

/// Locates the opencode trace-logger NDJSON for one agent session id.
///
/// The opencode trace-logger plugin writes one NDJSON file per session to
/// `~/.local/share/opencode/trace/<session-id>.ndjson`. The session id is the
/// agent's own `ses_xxx` identifier, which Ora resolves privately (ADR-0003), so
/// only this one file needs to be checked — no directory scan required.
pub fn resolve_opencode_trace(
    home_directory: &Path,
    agent_session_id: &str,
) -> Result<Option<PathBuf>, std::io::Error> {
    let trace_dir = home_directory
        .join(".local")
        .join("share")
        .join("opencode")
        .join("trace");
    let candidate = trace_dir.join(format!("{agent_session_id}.ndjson"));
    Ok(candidate.is_file().then_some(candidate))
}

/// Resolves the agent-written trace file for one locator, by agent family.
pub fn resolve_trace_file(
    agent_type: DashboardAgentType,
    home_directory: &Path,
    agent_session_id: &str,
) -> Result<Option<PathBuf>, std::io::Error> {
    match agent_type {
        DashboardAgentType::ClaudeCode => {
            resolve_claude_code_trace(home_directory, agent_session_id)
        }
        DashboardAgentType::Opencode => {
            resolve_opencode_trace(home_directory, agent_session_id)
        }
    }
}

/// Recursively yields files under `root` without depending on an external crate.
fn walkdir_files(root: &Path) -> std::io::Result<Box<dyn Iterator<Item = PathBuf>>> {
    let mut stack: Vec<PathBuf> = vec![root.to_path_buf()];
    let mut collected: Vec<PathBuf> = Vec::new();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                collected.push(path);
            }
        }
    }
    // Reverse so callers see the newest-scanned first; stability is by path.
    collected.reverse();
    Ok(Box::new(collected.into_iter()))
}

/// Builds the iframe URL Ora embeds for one dashboard session.
pub fn dashboard_url(
    host: &str,
    port: u16,
    ora_session_id: &str,
    agent_type: DashboardAgentType,
) -> String {
    format!(
        "http://{host}:{port}/?session_id={oid}&agent_type={at}",
        oid = urlencoding::encode(ora_session_id),
        at = agent_type.as_str(),
    )
}

/// Carries the empty request used to ask Ora for a dashboard iframe URL.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDashboardUrlRequest {
    pub session_id: String,
}

/// Returns the resolved dashboard endpoint and iframe URL for one Ora session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetDashboardUrlResponse {
    pub host: String,
    pub port: u16,
    pub url: String,
    /// True when the dashboard server answered the health probe on host:port.
    pub server_reachable: bool,
}

/// Resolves one Ora session to a dashboard iframe URL, writing the locator first.
#[tauri::command]
pub async fn get_dashboard_url(
    state: State<'_, DesktopState>,
    request: GetDashboardUrlRequest,
) -> Result<GetDashboardUrlResponse, CommandError> {
    let config = state.config.snapshot().map_err(CommandError::from)?;
    let host = config.dashboard_host().to_string();
    let port = config.dashboard_port();

    // Backend-only resolution; the agent session id stays in Rust and is never
    // placed in the URL (ADR-0003).
    let locator_info = state
        .backend
        .resolve_session_locator(&request.session_id)
        .map_err(CommandError::from)?;
    let agent_type = dashboard_agent_type(locator_info.agent_cli);

    let trace_file_path = match resolve_trace_file(
        agent_type,
        &locator_info.home_directory,
        &locator_info.agent_session_id,
    ) {
        Ok(Some(path)) => Some(path.to_string_lossy().into_owned()),
        Ok(None) => None,
        Err(_) => {
            return Err(CommandError::new(
                "dashboard_trace_resolution_error",
                "failed to scan the agent trace directory",
            ))
        }
    };

    let Some(trace_file_path) = trace_file_path else {
        return Err(CommandError::new(
            "dashboard_trace_not_ready",
            "trace file has not been produced yet for this session",
        ));
    };

    write_locator(
        &state.app_data_directory,
        &request.session_id,
        &DashboardLocator {
            trace_file_path,
            agent_type: agent_type.as_str().to_string(),
        },
    )
    .map_err(|_| {
        CommandError::new(
            "dashboard_locator_write_error",
            "failed to write the dashboard locator",
        )
    })?;

    let url = dashboard_url(&host, port, &request.session_id, agent_type);
    let server_reachable = probe_dashboard_server(&host, port).await;

    Ok(GetDashboardUrlResponse {
        host,
        port,
        url,
        server_reachable,
    })
}

/// Returns true when something answers a TCP connect on host:port within a beat.
async fn probe_dashboard_server(host: &str, port: u16) -> bool {
    use std::time::Duration;
    let addr = format!("{host}:{port}");
    tokio::time::timeout(Duration::from_millis(500), tokio::net::TcpStream::connect(&addr))
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    /// Verifies each persisted AgentCli maps to its dashboard agent family.
    #[test]
    fn normalizes_agent_cli_to_dashboard_family() {
        assert_eq!(
            dashboard_agent_type(ContractAgentCli::OpenCode),
            DashboardAgentType::Opencode
        );
        assert_eq!(
            dashboard_agent_type(ContractAgentCli::Nga),
            DashboardAgentType::Opencode
        );
        assert_eq!(
            dashboard_agent_type(ContractAgentCli::CodeAgentCli),
            DashboardAgentType::ClaudeCode
        );
    }

    /// Verifies the iframe URL carries only the Ora session id and canonical agent type.
    #[test]
    fn builds_dashboard_url_with_ora_session_id_only() {
        let url = dashboard_url(
            "127.0.0.1",
            8601,
            "811983f7-b35f-49a1-91ca-378ef1ece7ac",
            DashboardAgentType::ClaudeCode,
        );
        assert_eq!(
            url,
            "http://127.0.0.1:8601/?session_id=811983f7-b35f-49a1-91ca-378ef1ece7ac&agent_type=claude_code"
        );
    }

    /// Verifies the locator is written as camelCase JSON at the conventional path.
    #[test]
    fn writes_locator_at_conventional_path() {
        let temporary = TempDir::new().expect("create temporary app data directory");
        let app_data = temporary.path().to_path_buf();
        let locator = DashboardLocator {
            trace_file_path: "/tmp/trace.jsonl".to_string(),
            agent_type: "opencode".to_string(),
        };
        write_locator(&app_data, "sess-1", &locator).expect("write locator");

        let persisted = fs::read_to_string(DashboardLocator::file_for(&app_data, "sess-1"))
            .expect("read persisted locator");
        assert!(persisted.contains("\"traceFilePath\""));
        assert!(persisted.contains("\"agentType\""));
        assert!(persisted.contains("/tmp/trace.jsonl"));
        assert!(persisted.contains("\"opencode\""));
    }

    /// Verifies the locator directory and file paths are derived from app data dir.
    #[test]
    fn locator_paths_are_under_app_data_dashboard() {
        let app_data = PathBuf::from("/ora/app-data");
        assert_eq!(
            DashboardLocator::directory_for(&app_data),
            PathBuf::from("/ora/app-data/dashboard")
        );
        assert_eq!(
            DashboardLocator::file_for(&app_data, "abc"),
            PathBuf::from("/ora/app-data/dashboard/abc.json")
        );
    }

    /// Verifies Claude transcript discovery matches by file name across project dirs.
    #[test]
    fn resolves_claude_code_trace_by_agent_session_id_filename() {
        let temporary = TempDir::new().expect("create temporary home directory");
        let home = temporary.path().to_path_buf();
        // Two project-hash directories, each with a different session file.
        let projects = home.join(".claude").join("projects");
        let p1 = projects.join("hash-one");
        let p2 = projects.join("hash-two");
        fs::create_dir_all(&p1).expect("create project one");
        fs::create_dir_all(&p2).expect("create project two");
        fs::write(p1.join("other-session.jsonl"), b"{}").expect("write other session");
        fs::write(p2.join("target-session.jsonl"), b"{}").expect("write target session");

        let resolved = resolve_claude_code_trace(&home, "target-session")
            .expect("scan transcripts")
            .expect("target session should be found");
        assert_eq!(
            resolved,
            p2.join("target-session.jsonl")
        );

        assert!(
            resolve_claude_code_trace(&home, "missing")
                .expect("scan transcripts")
                .is_none(),
            "missing session resolves to None"
        );
    }

    /// Verifies a missing ~/.claude/projects directory resolves to None without error.
    #[test]
    fn resolves_claude_code_trace_none_without_projects_dir() {
        let temporary = TempDir::new().expect("create temporary home directory");
        let home = temporary.path().to_path_buf();
        assert!(
            resolve_claude_code_trace(&home, "any")
                .expect("scan transcripts")
                .is_none()
        );
    }

    /// Verifies the opencode trace-logger file is resolved by its single known path.
    #[test]
    fn resolves_opencode_trace_at_conventional_path() {
        let temporary = TempDir::new().expect("create temporary home directory");
        let home = temporary.path().to_path_buf();
        let trace_dir = home
            .join(".local")
            .join("share")
            .join("opencode")
            .join("trace");
        fs::create_dir_all(&trace_dir).expect("create trace directory");
        fs::write(trace_dir.join("ses_abc.ndjson"), b"{}")
            .expect("write trace file");

        let resolved = resolve_opencode_trace(&home, "ses_abc")
            .expect("resolve opencode trace")
            .expect("trace file should be found");
        assert_eq!(resolved, trace_dir.join("ses_abc.ndjson"));

        assert!(
            resolve_opencode_trace(&home, "ses_missing")
                .expect("resolve opencode trace")
                .is_none(),
            "missing opencode trace resolves to None"
        );
    }

    /// Verifies the trace dispatcher routes each agent family to its resolver.
    #[test]
    fn resolve_trace_file_dispatches_by_agent_family() {
        let temporary = TempDir::new().expect("create temporary home directory");
        let home = temporary.path().to_path_buf();

        // opencode trace file present.
        let oc_dir = home.join(".local").join("share").join("opencode").join("trace");
        fs::create_dir_all(&oc_dir).expect("create opencode trace dir");
        fs::write(oc_dir.join("ses_oc.ndjson"), b"{}").expect("write opencode trace");
        let oc = resolve_trace_file(DashboardAgentType::Opencode, &home, "ses_oc")
            .expect("dispatch opencode")
            .expect("opencode trace should be found");
        assert_eq!(oc, oc_dir.join("ses_oc.ndjson"));

        // claude transcript present.
        let cc_dir = home.join(".claude").join("projects").join("hash-one");
        fs::create_dir_all(&cc_dir).expect("create claude project dir");
        fs::write(cc_dir.join("cc-sess.jsonl"), b"{}").expect("write claude transcript");
        let cc = resolve_trace_file(DashboardAgentType::ClaudeCode, &home, "cc-sess")
            .expect("dispatch claude")
            .expect("claude transcript should be found");
        assert_eq!(cc, cc_dir.join("cc-sess.jsonl"));
    }
}
