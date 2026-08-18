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
use ora_backend::{BackendError, ErrorClassification};
use ora_contracts::{EmptyErrorParams, PublicError};
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

/// Normalizes one built-in agent's persisted identity into the dashboard agent family.
///
/// Returns `None` outside Ora's five bundled CLIs: which agents exist is no longer fixed at
/// build time, and a plugin-provided agent has no known trace format for the dashboard to render.
pub fn dashboard_agent_type(agent_ref: &str) -> Option<DashboardAgentType> {
    match agent_ref {
        "ora-space.opencode" | "ora-space.nga" => Some(DashboardAgentType::Opencode),
        // Claude, CodeAgentCli, and Codex all emit Claude-Code-compatible transcripts
        // (JSONL / stream-json), so the claude_code parser covers them until a dedicated
        // Codex trace format is introduced.
        "ora-space.codeagentcli" | "ora-space.claude" | "ora-space.codex" => {
            Some(DashboardAgentType::ClaudeCode)
        }
        _ => None,
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
    ///
    /// The session id is expected to be a backend-resolved UUID; this rejects
    /// any id containing path separators or traversal segments so a malicious or
    /// malformed id can never escape the dashboard directory.
    pub fn file_for(app_data_directory: &Path, ora_session_id: &str) -> PathBuf {
        debug_assert_path_safe(ora_session_id);
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
    write!(
        temp,
        "{}",
        serde_json::to_string_pretty(locator).map_err(std::io::Error::other)?
    )?;
    temp.write_all(b"\n")?;
    temp.as_file().sync_all()?;
    temp.persist(&target).map(|_| ()).map_err(|e| e.error)
}

/// Locates the Claude Code transcript JSONL for one agent session id.
///
/// Every Claude-Code-compatible fork auto-saves interactive sessions to
/// `<home>/<config>/projects/<project-hash>/<session-id>.jsonl`, but the config
/// directory is fork-specific: stock Claude Code and Codex use `.claude`, while
/// the `codeagentcli` fork uses `.cac` (its data directory, distinct from the
/// `~/.codeagentcli` binary install dir). Ora launches these CLIs over stdio
/// without passing a config dir, so each fork's default must be resolved here.
/// The project hash is derived from the working directory and is not worth
/// recomputing, so this scans every project directory under the CLI's projects
/// root and matches by file name (the agent session id).
pub fn resolve_claude_code_trace(
    agent_ref: &str,
    home_directory: &Path,
    agent_session_id: &str,
) -> Result<Option<PathBuf>, std::io::Error> {
    let config_directory = match agent_ref {
        "ora-space.codeagentcli" => ".cac",
        // Stock Claude and Codex use `.claude`. Any other identity reaching this function is a
        // caller bug (the opencode family and unrecognized agents are filtered out by
        // `dashboard_agent_type` before this is called), so it falls back to the common default
        // rather than being rejected here.
        _ => ".claude",
    };
    let projects_root = home_directory.join(config_directory).join("projects");
    if !projects_root.is_dir() {
        return Ok(None);
    }
    let target_name = format!("{agent_session_id}.jsonl");
    for path in walkdir_files(&projects_root) {
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

/// Resolves the agent-written trace file for one session, by agent family.
///
/// Dispatches on the dashboard agent family derived from `agent_cli`. The
/// Claude-Code family additionally needs the full CLI identity to pick its
/// fork-specific projects root, so the contract CLI is threaded in directly
/// rather than the collapsed `DashboardAgentType`.
pub fn resolve_trace_file(
    agent_ref: &str,
    home_directory: &Path,
    agent_session_id: &str,
) -> Result<Option<PathBuf>, std::io::Error> {
    match dashboard_agent_type(agent_ref) {
        Some(DashboardAgentType::ClaudeCode) => {
            resolve_claude_code_trace(agent_ref, home_directory, agent_session_id)
        }
        Some(DashboardAgentType::Opencode) => {
            resolve_opencode_trace(home_directory, agent_session_id)
        }
        // No known trace format for this agent; the caller reports that distinctly from a known
        // format whose file simply has not appeared yet.
        None => Ok(None),
    }
}

/// Recursively yields files under `root` without depending on an external crate.
///
/// Returns a truly lazy iterator so callers that find their target early (e.g.
/// `resolve_claude_code_trace`) can short-circuit without reading the whole tree.
/// Unreadable subdirectories are logged and skipped so one permission issue does
/// not abort the entire scan.
fn walkdir_files(root: &Path) -> WalkDirIter {
    WalkDirIter {
        stack: vec![root.to_path_buf()],
    }
}

/// Stateful depth-first file iterator backed by a path stack.
struct WalkDirIter {
    stack: Vec<PathBuf>,
}

impl Iterator for WalkDirIter {
    type Item = PathBuf;

    fn next(&mut self) -> Option<PathBuf> {
        while let Some(path) = self.stack.pop() {
            if path.is_dir() {
                match std::fs::read_dir(&path) {
                    Ok(entries) => self
                        .stack
                        .extend(entries.flatten().map(|entry| entry.path())),
                    Err(error) => {
                        tracing::warn!(
                            ?path,
                            ?error,
                            "skipping unreadable directory during trace scan"
                        );
                    }
                }
            } else {
                return Some(path);
            }
        }
        None
    }
}

/// Asserts an Ora session id cannot escape its locator directory via path traversal.
fn debug_assert_path_safe(id: &str) {
    debug_assert!(
        !id.contains('/') && !id.contains('\\') && !id.contains(".."),
        "ora_session_id must not contain path separators or traversal segments: {id:?}",
    );
}

/// Builds the iframe URL Ora embeds for one dashboard session.
pub fn dashboard_url(
    host: &str,
    port: u16,
    ora_session_id: &str,
    agent_type: DashboardAgentType,
) -> String {
    let authority = format_host_port(host, port);
    format!(
        "http://{authority}/?session_id={oid}&agent_type={at}",
        oid = urlencoding::encode(ora_session_id),
        at = agent_type.as_str(),
    )
}

/// Builds the iframe URL for the standalone token-comparison dashboard mode.
pub fn dashboard_compare_url(host: &str, port: u16) -> String {
    let authority = format_host_port(host, port);
    format!("http://{authority}/?app_mode=compare")
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
    let Some(agent_type) = dashboard_agent_type(&locator_info.agent_ref) else {
        return Err(CommandError::from_backend(BackendError::new(
            ErrorClassification::Internal,
            PublicError::InternalError(EmptyErrorParams {}),
            "this agent has no known trace format for the dashboard",
        )));
    };

    let trace_file_path = match resolve_trace_file(
        &locator_info.agent_ref,
        &locator_info.home_directory,
        &locator_info.agent_session_id,
    ) {
        Ok(Some(path)) => Some(path.to_string_lossy().into_owned()),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(?err, "failed to scan the agent trace directory");
            return Err(CommandError::from_backend(BackendError::new(
                ErrorClassification::Internal,
                PublicError::InternalError(EmptyErrorParams {}),
                "failed to scan the agent trace directory",
            )));
        }
    };

    let Some(trace_file_path) = trace_file_path else {
        return Err(CommandError::from_backend(BackendError::new(
            ErrorClassification::Internal,
            PublicError::InternalError(EmptyErrorParams {}),
            "trace file has not been produced yet for this session",
        )));
    };

    write_locator(
        &state.app_data_directory,
        &request.session_id,
        &DashboardLocator {
            trace_file_path,
            agent_type: agent_type.as_str().to_string(),
        },
    )
    .map_err(|err| {
        tracing::warn!(?err, "failed to write the dashboard locator");
        CommandError::from_backend(BackendError::new(
            ErrorClassification::Internal,
            PublicError::InternalError(EmptyErrorParams {}),
            "failed to write the dashboard locator",
        ))
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

/// Returns the token-comparison dashboard endpoint without resolving a session trace.
#[tauri::command]
pub async fn get_dashboard_compare_url(
    state: State<'_, DesktopState>,
) -> Result<GetDashboardUrlResponse, CommandError> {
    let config = state.config.snapshot().map_err(CommandError::from)?;
    let host = config.dashboard_host().to_string();
    let port = config.dashboard_port();
    let url = dashboard_compare_url(&host, port);
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
    let addr = format_host_port(host, port);
    tokio::time::timeout(
        Duration::from_millis(500),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    .map(|r| r.is_ok())
    .unwrap_or(false)
}

/// Formats a host:port pair, wrapping IPv6 hosts in brackets per RFC 3986.
fn format_host_port(host: &str, port: u16) -> String {
    let is_ipv6 = host.contains(':') && !host.starts_with('[');
    if is_ipv6 {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    /// Verifies each built-in agent identity maps to its dashboard agent family.
    #[test]
    fn normalizes_agent_ref_to_dashboard_family() {
        assert_eq!(
            dashboard_agent_type("ora-space.opencode"),
            Some(DashboardAgentType::Opencode)
        );
        assert_eq!(
            dashboard_agent_type("ora-space.nga"),
            Some(DashboardAgentType::Opencode)
        );
        assert_eq!(
            dashboard_agent_type("ora-space.codeagentcli"),
            Some(DashboardAgentType::ClaudeCode)
        );
        assert_eq!(
            dashboard_agent_type("ora-space.claude"),
            Some(DashboardAgentType::ClaudeCode)
        );
        assert_eq!(
            dashboard_agent_type("ora-space.codex"),
            Some(DashboardAgentType::ClaudeCode)
        );
    }

    /// Verifies an agent this build does not recognize gets no dashboard trace format.
    #[test]
    fn plugin_provided_agent_has_no_known_dashboard_family() {
        assert_eq!(dashboard_agent_type("acme.my-agent"), None);
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

    /// Verifies compare mode carries only the app mode and no trace/session path.
    #[test]
    fn builds_dashboard_compare_url_without_trace_path() {
        let url = dashboard_compare_url("127.0.0.1", 8601);
        assert_eq!(url, "http://127.0.0.1:8601/?app_mode=compare");
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

        let resolved = resolve_claude_code_trace("ora-space.claude", &home, "target-session")
            .expect("scan transcripts")
            .expect("target session should be found");
        assert_eq!(resolved, p2.join("target-session.jsonl"));

        assert!(
            resolve_claude_code_trace("ora-space.claude", &home, "missing")
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
            resolve_claude_code_trace("ora-space.claude", &home, "any")
                .expect("scan transcripts")
                .is_none()
        );
    }

    /// Verifies codeagentcli transcripts are discovered under ~/.cac/projects, not
    /// ~/.claude/projects, so the fork's separate data directory is respected.
    #[test]
    fn resolves_codeagentcli_trace_under_cac_projects_root() {
        let temporary = TempDir::new().expect("create temporary home directory");
        let home = temporary.path().to_path_buf();
        // codeagentcli writes under .cac; stock Claude would write under .claude.
        // Both hold a same-named file to prove each CLI scans only its own root.
        let cac_projects = home.join(".cac").join("projects").join("hash");
        let claude_projects = home.join(".claude").join("projects").join("hash");
        fs::create_dir_all(&cac_projects).expect("create codeagentcli project dir");
        fs::create_dir_all(&claude_projects).expect("create stock claude project dir");
        for sibling in ["older-session.jsonl", "newer-session.jsonl"] {
            fs::write(cac_projects.join(sibling), b"{}")
                .expect("write sibling codeagentcli transcript");
        }
        fs::write(cac_projects.join("xc-session.jsonl"), b"{}")
            .expect("write codeagentcli transcript");
        fs::write(claude_projects.join("xc-session.jsonl"), b"{}")
            .expect("write stock claude transcript");

        let resolved = resolve_claude_code_trace("ora-space.codeagentcli", &home, "xc-session")
            .expect("scan transcripts")
            .expect("codeagentcli session should be found under .cac");
        assert_eq!(resolved, cac_projects.join("xc-session.jsonl"));

        // Stock Claude must still resolve from .claude, proving the two CLIs are
        // isolated by their fork-specific projects root.
        let claude_resolved = resolve_claude_code_trace("ora-space.claude", &home, "xc-session")
            .expect("scan transcripts")
            .expect("stock claude session should be found under .claude");
        assert_eq!(claude_resolved, claude_projects.join("xc-session.jsonl"));
    }

    /// Verifies transcript discovery does not discard sibling files after yielding
    /// the first directory entry, because one project normally owns many sessions.
    #[test]
    fn walks_every_trace_file_in_the_same_project_directory() {
        let temporary = TempDir::new().expect("create temporary projects directory");
        let project = temporary.path().join("project-hash");
        fs::create_dir_all(&project).expect("create project directory");
        let mut expected = vec![
            project.join("session-one.jsonl"),
            project.join("session-two.jsonl"),
            project.join("session-three.jsonl"),
        ];
        for path in &expected {
            fs::write(path, b"{}").expect("write session transcript");
        }

        let mut actual = walkdir_files(temporary.path()).collect::<Vec<_>>();
        expected.sort();
        actual.sort();
        assert_eq!(actual, expected);
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
        fs::write(trace_dir.join("ses_abc.ndjson"), b"{}").expect("write trace file");

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

    /// Verifies the trace dispatcher routes each agent family and CLI to its resolver.
    #[test]
    fn resolve_trace_file_dispatches_by_agent_family() {
        let temporary = TempDir::new().expect("create temporary home directory");
        let home = temporary.path().to_path_buf();

        // opencode trace file present.
        let oc_dir = home
            .join(".local")
            .join("share")
            .join("opencode")
            .join("trace");
        fs::create_dir_all(&oc_dir).expect("create opencode trace dir");
        fs::write(oc_dir.join("ses_oc.ndjson"), b"{}").expect("write opencode trace");
        let oc = resolve_trace_file("ora-space.opencode", &home, "ses_oc")
            .expect("dispatch opencode")
            .expect("opencode trace should be found");
        assert_eq!(oc, oc_dir.join("ses_oc.ndjson"));

        // stock claude transcript present under .claude.
        let cc_dir = home.join(".claude").join("projects").join("hash-one");
        fs::create_dir_all(&cc_dir).expect("create claude project dir");
        fs::write(cc_dir.join("cc-sess.jsonl"), b"{}").expect("write claude transcript");
        let cc = resolve_trace_file("ora-space.claude", &home, "cc-sess")
            .expect("dispatch claude")
            .expect("claude transcript should be found");
        assert_eq!(cc, cc_dir.join("cc-sess.jsonl"));

        // codeagentcli transcript present under .cac (its fork-specific data dir).
        let xc_dir = home.join(".cac").join("projects").join("hash-two");
        fs::create_dir_all(&xc_dir).expect("create codeagentcli project dir");
        fs::write(xc_dir.join("xc-sess.jsonl"), b"{}").expect("write codeagentcli transcript");
        let xc = resolve_trace_file("ora-space.codeagentcli", &home, "xc-sess")
            .expect("dispatch codeagentcli")
            .expect("codeagentcli transcript should be found");
        assert_eq!(xc, xc_dir.join("xc-sess.jsonl"));
    }

    /// Verifies the IPv6 loopback host is bracketed in the dashboard URL.
    #[test]
    fn brackets_ipv6_loopback_in_dashboard_url() {
        let url = dashboard_url("::1", 8601, "sess-1", DashboardAgentType::Opencode);
        assert!(url.starts_with("http://[::1]:8601/"), "got {url}");
    }

    /// Verifies format_host_port wraps IPv6 but leaves IPv4 untouched.
    #[test]
    fn formats_host_port_with_ipv6_brackets() {
        assert_eq!(format_host_port("127.0.0.1", 8601), "127.0.0.1:8601");
        assert_eq!(format_host_port("::1", 8601), "[::1]:8601");
        assert_eq!(format_host_port("[::1]", 8601), "[::1]:8601");
    }

    /// Verifies probe_dashboard_server returns false for a port with no listener.
    #[tokio::test]
    async fn probe_returns_false_for_closed_port() {
        // Bind + immediately drop to get a guaranteed-free port.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind temporary listener");
        let free_port = listener.local_addr().expect("local addr").port();
        drop(listener);
        assert!(!probe_dashboard_server("127.0.0.1", free_port).await);
    }

    /// Verifies probe_dashboard_server returns true when a local listener answers.
    #[tokio::test]
    async fn probe_returns_true_for_open_port() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind temporary listener");
        let port = listener.local_addr().expect("local addr").port();
        // Accept in the background so the probe's connect succeeds.
        tokio::spawn(async move {
            let _ = listener.accept().await;
        });
        assert!(probe_dashboard_server("127.0.0.1", port).await);
    }
}
