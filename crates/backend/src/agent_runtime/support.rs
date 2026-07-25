use crate::{BackendError, BackendErrorKind};
use gitlancer::git::worktree::ResolveWorktreeByBranchRequest;
use gitlancer::{CliGitRunner, Git, RepoRoot, Repository};
use ora_acp::AcpClient;
use ora_application::{ProjectRepository, TaskRepository, WorktreeRepository};
use ora_contracts::acp::permission::{
    PermissionOptionId, RequestPermissionOutcome, RequestPermissionResponse,
    SelectedPermissionOutcome,
};
use ora_contracts::{
    AgentCli as ContractAgentCli, RespondToPermissionRequest, RespondToPermissionResponse,
    Session as ContractSession, SessionStatus as ContractSessionStatus,
};
use ora_db::{
    RepositoryPool, SqliteProjectRepository, SqliteTaskRepository, SqliteWorktreeRepository,
};
use ora_domain::{AgentCli, Session, SessionStatus, TaskId, WorktreeActivity};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::ChildStdin;

/// Resolves the task's authoritative execution directory from its selected workspace mode.
pub(crate) fn resolve_task_cwd(
    pool: &RepositoryPool,
    task_id: &TaskId,
) -> Result<PathBuf, BackendError> {
    let task = SqliteTaskRepository::new(pool.clone())
        .find_task(task_id)
        .map_err(|_| task_worktree_unavailable())?
        .ok_or_else(task_worktree_unavailable)?;
    if task.worktree_id.is_none() {
        let project = SqliteProjectRepository::new(pool.clone())
            .find_project(&task.project_id)
            .map_err(|_| task_project_root_unavailable())?
            .ok_or_else(task_project_root_unavailable)?;
        let cwd = absolute_project_root(PathBuf::from(project.root_path))?;
        return if cwd.is_dir() {
            Ok(cwd)
        } else {
            Err(task_project_root_unavailable())
        };
    }

    let worktree_id = task.worktree_id.ok_or_else(task_worktree_unavailable)?;
    let worktree = SqliteWorktreeRepository::new(pool.clone())
        .find_worktree(&worktree_id)
        .map_err(|_| task_worktree_unavailable())?
        .ok_or_else(task_worktree_unavailable)?;
    if worktree.task_id != task.id || worktree.activity != WorktreeActivity::Active {
        return Err(task_worktree_unavailable());
    }
    let branch_name = worktree.branch_name.ok_or_else(task_worktree_unavailable)?;
    let project = SqliteProjectRepository::new(pool.clone())
        .find_project(&task.project_id)
        .map_err(|_| task_worktree_unavailable())?
        .ok_or_else(task_worktree_unavailable)?;
    let repository = Repository::new(RepoRoot::new(project.root_path));
    let resolved = Git::new(CliGitRunner)
        .resolve_worktree_by_branch(ResolveWorktreeByBranchRequest {
            repository: &repository,
            branch_name: &branch_name,
        })
        .map_err(|_| task_worktree_unavailable())?;
    let cwd = resolved.worktree_root().as_path().to_path_buf();
    if !cwd.is_dir() {
        return Err(task_worktree_unavailable());
    }
    Ok(cwd)
}

/// Normalizes a stored project root before it crosses the ACP process boundary.
///
/// Relative project roots remain valid in persisted server configurations, while providers
/// require a stable absolute working directory after Ora starts them.
fn absolute_project_root(path: PathBuf) -> Result<PathBuf, BackendError> {
    if path.is_absolute() {
        return Ok(path);
    }
    std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .map_err(|_| task_project_root_unavailable())
}

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

/// Builds the conflict used when task ownership cannot resolve an active Git worktree.
fn task_worktree_unavailable() -> BackendError {
    BackendError::new(
        BackendErrorKind::Conflict,
        "task_worktree_unavailable",
        "task worktree is unavailable",
    )
}

/// Builds the conflict used when a project-root task no longer has a usable directory.
fn task_project_root_unavailable() -> BackendError {
    BackendError::new(
        BackendErrorKind::Conflict,
        "task_project_root_unavailable",
        "task project root is unavailable",
    )
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::resolve_agent_cli_path;
    use super::{absolute_project_root, resolve_task_cwd};
    use ora_application::{ProjectRepository, TaskRepository};
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, SqliteProjectRepository, SqliteTaskRepository,
        default_migration_catalog,
    };
    #[cfg(unix)]
    use ora_domain::AgentCli;
    use ora_domain::{AuditFields, Project, ProjectId, Task, TaskId, TaskStatus};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

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

    /// Verifies direct-chat tasks start providers in the project root without a worktree link.
    #[test]
    fn resolves_project_root_for_tasks_without_worktrees() {
        let temp_dir = TempDir::new().expect("create temporary directory");
        let project_root = temp_dir.path().join("project-root");
        fs::create_dir_all(&project_root).expect("create project root");
        let database_path = temp_dir.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&database_path),
                &default_migration_catalog().expect("create migration catalog"),
            )
            .expect("bootstrap repository pool");
        SqliteProjectRepository::new(pool.clone())
            .create_project(Project::new(
                ProjectId::new("project-1"),
                "Project",
                project_root.to_string_lossy(),
                AuditFields::new(1, 1, false),
            ))
            .expect("persist project");
        SqliteTaskRepository::new(pool.clone())
            .create_task(Task::new(
                TaskId::new("task-1"),
                ProjectId::new("project-1"),
                "Project chat",
                TaskStatus::Doing,
                None,
                AuditFields::new(1, 1, false),
            ))
            .expect("persist task");

        assert_eq!(
            resolve_task_cwd(&pool, &TaskId::new("task-1")).expect("resolve project root cwd"),
            project_root,
        );
    }

    /// Verifies relative roots are made stable before being passed to provider processes.
    #[test]
    fn normalizes_relative_project_roots_for_acp() {
        let cwd = absolute_project_root(PathBuf::from(".")).expect("resolve relative project root");
        assert!(cwd.is_absolute());
        assert!(cwd.is_dir());
    }
}
