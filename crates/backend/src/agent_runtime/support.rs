use crate::{BackendError, BackendErrorKind};
use ora_acp::AcpClient;
use ora_contracts::acp::permission::{
    PermissionOptionId, RequestPermissionOutcome, RequestPermissionResponse,
    SelectedPermissionOutcome,
};
use ora_contracts::{
    AgentCli as ContractAgentCli, RespondToPermissionRequest, RespondToPermissionResponse,
    Session as ContractSession, SessionStatus as ContractSessionStatus,
};
use ora_domain::{AgentCli, Session, SessionStatus};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::ChildStdin;

/// Responds to a pending permission after validating the public request ownership.
pub(super) async fn respond_permission(
    client: &AcpClient<ChildStdin>,
    request: RespondToPermissionRequest,
    permissions: &mut HashMap<String, (ora_contracts::acp::rpc::RequestId, Vec<String>)>,
) -> Result<RespondToPermissionResponse, BackendError> {
    let Some((request_id, options)) = permissions.remove(&request.permission_request_id) else {
        return Err(BackendError::new(
            BackendErrorKind::Conflict,
            "permission_request_not_pending",
            "permission request is not pending",
        ));
    };
    if !options.contains(&request.option_id) {
        permissions.insert(request.permission_request_id, (request_id, options));
        return Err(BackendError::new(
            BackendErrorKind::BadRequest,
            "permission_option_invalid",
            "permission option does not belong to this request",
        ));
    }
    let outcome = RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
        PermissionOptionId::new(request.option_id),
    ));
    client
        .respond(&request_id, &RequestPermissionResponse::new(outcome))
        .await
        .map_err(map_acp_error)?;
    Ok(RespondToPermissionResponse {})
}

/// Maps a private domain session into its frontend-safe view.
pub(super) fn contract_session(session: Session) -> ContractSession {
    ContractSession {
        id: session.id.to_string(),
        task_id: session.task_id.to_string(),
        agent_cli: contract_agent_cli(session.agent_cli),
        status: match session.status {
            SessionStatus::Running => ContractSessionStatus::Running,
            SessionStatus::Stopped => ContractSessionStatus::Stopped,
        },
    }
}

/// Maps the stable persisted CLI identity into its transport representation.
pub(super) fn contract_agent_cli(agent_cli: AgentCli) -> ContractAgentCli {
    match agent_cli {
        AgentCli::OpenCode => ContractAgentCli::OpenCode,
        AgentCli::Nga => ContractAgentCli::Nga,
        AgentCli::CodeAgentCli => ContractAgentCli::CodeAgentCli,
    }
}

/// Resolves one CLI through the Windows executable lookup mechanism for each retry generation.
#[cfg(windows)]
pub(super) fn resolve_agent_cli_path(
    agent_cli: AgentCli,
    _home_directory: &Path,
) -> Result<PathBuf, BackendError> {
    let output = std::process::Command::new("where.exe")
        .arg(agent_cli.executable_name())
        .output()
        .map_err(|_| runtime_internal("agent_cli_resolution_failed", "failed to run where.exe"))?;
    if !output.status.success() {
        return Err(runtime_internal(
            "agent_cli_not_found",
            format!(
                "{} executable not found on PATH",
                agent_cli.executable_name()
            ),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find(|line| {
            let lower = line.to_lowercase();
            lower.ends_with(".exe") || lower.ends_with(".cmd") || lower.ends_with(".bat")
        })
        .or_else(|| stdout.lines().next())
        .map(|path| PathBuf::from(path.trim()))
        .ok_or_else(|| {
            runtime_internal(
                "agent_cli_not_found",
                format!(
                    "{} executable not found on PATH",
                    agent_cli.executable_name()
                ),
            )
        })
}

/// Resolves one CLI from its fixed per-user Unix installation directory.
#[cfg(unix)]
pub(super) fn resolve_agent_cli_path(
    agent_cli: AgentCli,
    home_directory: &Path,
) -> Result<PathBuf, BackendError> {
    let installation_directory = match agent_cli {
        AgentCli::OpenCode => ".opencode",
        AgentCli::Nga => ".nga",
        AgentCli::CodeAgentCli => ".codeagentcli",
    };
    Ok(home_directory
        .join(installation_directory)
        .join("bin")
        .join(agent_cli.executable_name()))
}

/// Drains child stderr so provider diagnostics can never block the shared process.
pub(super) async fn drain_stderr(mut stderr: tokio::process::ChildStderr) {
    use tokio::io::AsyncReadExt;
    let mut tail = Vec::with_capacity(64 * 1024);
    let mut buffer = [0_u8; 4096];
    loop {
        match stderr.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                tail.extend_from_slice(&buffer[..read]);
                if tail.len() > 64 * 1024 {
                    tail.drain(..tail.len() - 64 * 1024);
                }
            }
        }
    }
}

/// Builds the stable public error for an unknown or deleted Ora session.
pub(super) fn session_not_found(session_id: &str) -> BackendError {
    BackendError::new(
        BackendErrorKind::NotFound,
        "session_not_found",
        format!("session not found: {session_id}"),
    )
}

/// Builds the conflict returned when a prompt targets an unloaded logical session.
pub(super) fn session_stopped() -> BackendError {
    BackendError::new(
        BackendErrorKind::Conflict,
        "session_stopped",
        "session must be loaded before prompting",
    )
}

/// Builds the degraded-mode error while the selected CLI is starting or recovering.
pub(super) fn runtime_unavailable() -> BackendError {
    runtime_internal(
        "agent_runtime_unavailable",
        "agent CLI runtime is unavailable",
    )
}

/// Hides transport internals behind the backend's stable protocol error.
pub(super) fn map_acp_error(error: ora_acp::AcpError) -> BackendError {
    runtime_internal("agent_protocol_error", error.to_string())
}

/// Builds an internal runtime error with a caller-selected stable code.
pub(super) fn runtime_internal(code: &'static str, message: impl Into<String>) -> BackendError {
    BackendError::new(BackendErrorKind::Internal, code, message)
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::resolve_agent_cli_path;
    #[cfg(unix)]
    use ora_domain::AgentCli;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    /// Verifies Unix lookup remains relative to the injected user home.
    #[cfg(unix)]
    #[test]
    fn resolves_unix_cli_paths_from_home_directory() {
        let home_directory = PathBuf::from("users").join("demo");
        assert_eq!(
            AgentCli::ALL.map(|agent_cli| {
                resolve_agent_cli_path(agent_cli, &home_directory).expect("resolve agent CLI path")
            }),
            [
                home_directory
                    .join(".opencode")
                    .join("bin")
                    .join("opencode"),
                home_directory.join(".nga").join("bin").join("nga"),
                home_directory
                    .join(".codeagentcli")
                    .join("bin")
                    .join("codeagentcli"),
            ]
        );
    }
}
