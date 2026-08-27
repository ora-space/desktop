use crate::error::{BackendError, ErrorClassification};
use crate::task::resolve_workspace_cwd;
use ora_application::{
    CommitWorkspaceChangesHandler, GitWorkspaceDiffReader, GitWorkspaceGitWriter,
    PushWorkspaceBranchHandler, ReadWorkspaceDiffRequest, ReadWorkspaceDiffScope,
    WorkspaceDiffReader, WorkspaceDiffReaderError, WorktreeRepository,
};
use ora_contracts::{
    CommitWorkspaceChangesRequest, CommitWorkspaceChangesResponse, GetWorkspaceDiffRequest,
    GetWorkspaceDiffResponse, PushWorkspaceBranchRequest, PushWorkspaceBranchResponse,
    WorkspaceDiffScope,
};
use ora_contracts::{EmptyErrorParams, PublicError};
use ora_db::{RepositoryPool, SqliteWorkspaceRepository, SqliteWorktreeRepository};
use ora_domain::{Workspace, WorkspaceId, WorkspaceLocation};
use std::path::PathBuf;

/// Owns workspace-scoped Git review operations shared by the Web and Desktop adapters.
///
/// Serves both an isolated task worktree and a project's main checkout — both are `Workspace`
/// rows. The two differ only in whether a `Worktree` row is recorded for them; see
/// `ora_application::workspace_diff` for how that shapes verification.
pub(crate) struct WorkspaceDiffApi {
    pool: RepositoryPool,
    git_cleanup: crate::git_cleanup::GitCleanupHandle,
    relative_path_base: PathBuf,
}

impl WorkspaceDiffApi {
    /// Builds the shared workspace diff API from durable repositories and Git cleanup.
    pub(crate) fn new(
        pool: RepositoryPool,
        git_cleanup: crate::git_cleanup::GitCleanupHandle,
        relative_path_base: PathBuf,
    ) -> Self {
        Self {
            pool,
            git_cleanup,
            relative_path_base,
        }
    }

    /// Computes a diff from the exact directory used as the workspace's checkout.
    pub(crate) fn get_diff(
        &self,
        request: GetWorkspaceDiffRequest,
    ) -> Result<GetWorkspaceDiffResponse, BackendError> {
        let workspace_id = WorkspaceId::new(request.workspace_id);
        let workspace = self.load_workspace(&workspace_id)?;
        // Shared use lease: physical cleanup of this Workspace's checkout waits
        // for this read instead of removing the directory underneath it.
        let _worktree_use = self.git_cleanup.shared_worktree_use(workspace_id.as_ref());
        let repository_root = self.load_repository_root(&workspace)?;
        let cwd = resolve_workspace_cwd(&self.pool, &workspace_id, &self.relative_path_base)?;

        let base_commit_id = self.recorded_baseline(&workspace_id)?;
        let scope = map_diff_scope(request.scope);
        if base_commit_id.is_none() && scope_requires_baseline(scope) {
            return Err(baseline_unavailable());
        }
        let snapshot = GitWorkspaceDiffReader::new(repository_root)
            .read_workspace_diff(ReadWorkspaceDiffRequest {
                worktree_path: cwd,
                base_commit_id: base_commit_id.clone(),
                scope,
            })
            .map_err(map_diff_reader_error)?;

        Ok(GetWorkspaceDiffResponse {
            base_commit_id,
            head_commit_id: snapshot.head_commit_id,
            patch: snapshot.patch,
        })
    }

    /// Commits current changes in a workspace checkout, verified against its recorded branch
    /// when one is persisted (an isolated task worktree), or trusted as-is when none is (a
    /// project's main checkout).
    pub(crate) fn commit_changes(
        &self,
        request: CommitWorkspaceChangesRequest,
    ) -> Result<CommitWorkspaceChangesResponse, BackendError> {
        let workspace_id = WorkspaceId::new(request.workspace_id.clone());
        let workspace = self.load_workspace(&workspace_id)?;
        // Shared use lease: see get_diff; commits must not lose the checkout mid-write.
        let _worktree_use = self.git_cleanup.shared_worktree_use(workspace_id.as_ref());
        let (repository_root, worktree_path) = self.worktree_context(&workspace, &workspace_id)?;
        CommitWorkspaceChangesHandler::new(
            SqliteWorktreeRepository::new(self.pool.clone()),
            GitWorkspaceGitWriter::new(repository_root),
            worktree_path,
        )
        .handle(request)
        .map_err(BackendError::from)
    }

    /// Pushes one workspace checkout's branch, verified when its `Worktree` row is recorded.
    pub(crate) fn push_branch(
        &self,
        request: PushWorkspaceBranchRequest,
    ) -> Result<PushWorkspaceBranchResponse, BackendError> {
        let workspace_id = WorkspaceId::new(request.workspace_id.clone());
        let workspace = self.load_workspace(&workspace_id)?;
        // Shared use lease: see get_diff; pushes read the checkout's branch state.
        let _worktree_use = self.git_cleanup.shared_worktree_use(workspace_id.as_ref());
        let (repository_root, worktree_path) = self.worktree_context(&workspace, &workspace_id)?;
        PushWorkspaceBranchHandler::new(
            SqliteWorktreeRepository::new(self.pool.clone()),
            GitWorkspaceGitWriter::new(repository_root),
            worktree_path,
        )
        .handle(request)
        .map_err(BackendError::from)
    }

    /// Loads one visible workspace while keeping storage diagnostics behind the backend contract.
    fn load_workspace(&self, workspace_id: &WorkspaceId) -> Result<Workspace, BackendError> {
        SqliteWorkspaceRepository::new(self.pool.clone())
            .find_workspace(workspace_id)
            .map_err(workspace_diff_internal)?
            .ok_or_else(crate::task::workspace_unavailable)
    }

    /// Resolves a workspace's repository from its project's main workspace location.
    fn load_repository_root(&self, workspace: &Workspace) -> Result<PathBuf, BackendError> {
        let main_workspace = SqliteWorkspaceRepository::new(self.pool.clone())
            .find_main_workspace(&workspace.project_id)
            .map_err(workspace_diff_internal)?
            .ok_or_else(|| {
                BackendError::new(
                    ErrorClassification::NotFound,
                    PublicError::ProjectNotFound(EmptyErrorParams {}),
                    "project not found",
                )
            })?;
        let WorkspaceLocation::LocalFilesystem { path } = main_workspace.location else {
            return Err(BackendError::new(
                ErrorClassification::Conflict,
                PublicError::WorkspaceUnavailable(EmptyErrorParams {}),
                "workspace is unavailable",
            ));
        };
        crate::task::absolute_project_root(PathBuf::from(path), &self.relative_path_base)
    }

    /// Resolves the repository and checkout directory required by write operations.
    fn worktree_context(
        &self,
        workspace: &Workspace,
        workspace_id: &WorkspaceId,
    ) -> Result<(PathBuf, PathBuf), BackendError> {
        let repository_root = self.load_repository_root(workspace)?;
        let cwd = resolve_workspace_cwd(&self.pool, workspace_id, &self.relative_path_base)?;
        Ok((repository_root, cwd))
    }

    /// Returns the baseline commit recorded for this workspace's `Worktree` row, when one exists.
    fn recorded_baseline(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<String>, BackendError> {
        Ok(SqliteWorktreeRepository::new(self.pool.clone())
            .find_worktree(workspace_id)
            .map_err(workspace_diff_internal)?
            .and_then(|worktree| worktree.baseline.commit_id().map(str::to_string)))
    }
}

/// Maps the public Git layer selector into the application diff reader vocabulary.
fn map_diff_scope(scope: WorkspaceDiffScope) -> ReadWorkspaceDiffScope {
    match scope {
        WorkspaceDiffScope::Branch => ReadWorkspaceDiffScope::Branch,
        WorkspaceDiffScope::Unstaged => ReadWorkspaceDiffScope::Unstaged,
        WorkspaceDiffScope::Staged => ReadWorkspaceDiffScope::Staged,
        WorkspaceDiffScope::Committed => ReadWorkspaceDiffScope::Committed,
    }
}

/// Reports whether a scope compares against a fixed baseline commit.
fn scope_requires_baseline(scope: ReadWorkspaceDiffScope) -> bool {
    matches!(
        scope,
        ReadWorkspaceDiffScope::Branch | ReadWorkspaceDiffScope::Committed
    )
}

/// Builds the public error for a `Branch`/`Committed` request with no recorded baseline.
fn baseline_unavailable() -> BackendError {
    BackendError::new(
        ErrorClassification::Conflict,
        PublicError::WorkspaceDiffBaselineUnavailable(EmptyErrorParams {}),
        "workspace diff baseline is unavailable",
    )
}

/// Preserves the public size limit while hiding Git execution diagnostics.
fn map_diff_reader_error(error: WorkspaceDiffReaderError) -> BackendError {
    match error {
        WorkspaceDiffReaderError::TooLarge { .. } => BackendError::new(
            ErrorClassification::PayloadTooLarge,
            PublicError::WorkspaceDiffTooLarge(EmptyErrorParams {}),
            "workspace diff exceeds the response limit",
        ),
        WorkspaceDiffReaderError::OperationFailed(source) => {
            BackendError::internal_boxed("workspace diff operation failed", source)
        }
    }
}

/// Retains repository diagnostics while exposing only the transport-neutral internal error.
fn workspace_diff_internal(source: impl std::error::Error + Send + Sync + 'static) -> BackendError {
    BackendError::internal("workspace diff operation failed", source)
}

#[cfg(test)]
mod tests {
    use crate::{Backend, BackendPaths};
    use ora_contracts::{
        CommitWorkspaceChangesRequest, CreateProjectRequest, CreateTaskRequest, EmptyErrorParams,
        GetWorkspaceDiffRequest, ListWorkspacesRequest, PublicError, PushWorkspaceBranchRequest,
        WorkspaceDiffScope,
    };
    use ora_test_support::GitTestScaffold;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    /// Verifies isolated task edits are read from the exact cwd resolved for the agent session.
    #[test]
    fn captures_agent_changes_from_worktree_tasks() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let scaffold = GitTestScaffold::new("backend-workspace-diff-worktree")
            .expect("create Git test scaffold");
        scaffold
            .write_file(scaffold.repo_path(), "README.md", "ora backend test\n")
            .expect("write repository seed file");
        scaffold
            .stage_all_and_commit("initial")
            .expect("create repository seed commit");
        let repository_root = scaffold.repo_path();
        let backend = open_backend(&temporary);
        let project_id = create_project(&backend, &repository_root);
        let task = backend
            .create_task(CreateTaskRequest {
                project_id,
                title: "Isolated task".to_string(),
                base_branch: Some("main".to_string()),
            })
            .expect("create worktree task")
            .task;
        let agent_cwd = backend
            .resolve_task_cwd(&task.id)
            .expect("resolve agent cwd");

        fs::write(agent_cwd.join("agent-change.txt"), "captured\n")
            .expect("write agent worktree change");

        let response = backend
            .get_workspace_diff(GetWorkspaceDiffRequest {
                workspace_id: task.workspace_id,
                scope: WorkspaceDiffScope::Branch,
            })
            .expect("read worktree workspace diff");

        assert!(response.patch.contains("agent-change.txt"));
        assert!(response.patch.contains("+captured"));
    }

    /// Verifies a project's main checkout (no task, no `Worktree` row) can be diffed, committed,
    /// and pushed directly by its main workspace id — the surface the review panel's "变更"
    /// button now serves for a plain project as well as a task.
    #[test]
    fn reads_commits_and_pushes_a_project_main_checkout_without_a_task() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let scaffold = GitTestScaffold::new("backend-workspace-diff-project")
            .expect("create Git test scaffold");
        scaffold
            .write_file(scaffold.repo_path(), "README.md", "ora backend test\n")
            .expect("write repository seed file");
        scaffold
            .stage_all_and_commit("initial")
            .expect("create repository seed commit");
        // A local bare remote lets the push assertion exercise the real `git push` path
        // without any network access.
        let remote_path = scaffold.sandbox_root().join("remote.git");
        fs::create_dir_all(&remote_path).expect("create bare remote directory");
        scaffold
            .run_git_in(&remote_path, ["init", "--bare"])
            .expect("initialize bare remote");
        scaffold
            .run_git(["remote", "add", "origin", &remote_path.to_string_lossy()])
            .expect("register bare remote as origin");

        let backend = open_backend(&temporary);
        let project_id = create_project(&backend, scaffold.repo_path());
        let main_workspace_id = backend
            .list_workspaces(ListWorkspacesRequest {})
            .expect("list workspaces")
            .workspaces
            .into_iter()
            .find(|workspace| workspace.project_id == project_id)
            .expect("project has a main workspace")
            .id;

        fs::write(
            scaffold.repo_path().join("project-change.txt"),
            "captured\n",
        )
        .expect("write main checkout change");

        let diff = backend
            .get_workspace_diff(GetWorkspaceDiffRequest {
                workspace_id: main_workspace_id.clone(),
                scope: WorkspaceDiffScope::Unstaged,
            })
            .expect("read main checkout diff");
        assert_eq!(diff.base_commit_id, None);
        assert!(diff.patch.contains("project-change.txt"));

        backend
            .commit_workspace_changes(CommitWorkspaceChangesRequest {
                workspace_id: main_workspace_id.clone(),
                message: "project change".to_string(),
            })
            .expect("commit main checkout change");

        backend
            .push_workspace_branch(PushWorkspaceBranchRequest {
                workspace_id: main_workspace_id,
            })
            .expect("push main checkout branch");
    }

    /// Verifies the `Branch` scope, which requires a recorded baseline, is rejected for a
    /// workspace with no `Worktree` row instead of silently diffing against nothing.
    #[test]
    fn rejects_branch_scope_for_a_workspace_with_no_recorded_baseline() {
        let temporary = TempDir::new().expect("create temporary backend directory");
        let scaffold = GitTestScaffold::new("backend-workspace-diff-no-baseline")
            .expect("create Git test scaffold");
        scaffold
            .write_file(scaffold.repo_path(), "README.md", "ora backend test\n")
            .expect("write repository seed file");
        scaffold
            .stage_all_and_commit("initial")
            .expect("create repository seed commit");
        let backend = open_backend(&temporary);
        let project_id = create_project(&backend, scaffold.repo_path());
        let main_workspace_id = backend
            .list_workspaces(ListWorkspacesRequest {})
            .expect("list workspaces")
            .workspaces
            .into_iter()
            .find(|workspace| workspace.project_id == project_id)
            .expect("project has a main workspace")
            .id;

        let error = backend
            .get_workspace_diff(GetWorkspaceDiffRequest {
                workspace_id: main_workspace_id,
                scope: WorkspaceDiffScope::Branch,
            })
            .expect_err("branch scope requires a baseline no main checkout has");

        assert_eq!(
            error.public_error().clone(),
            PublicError::WorkspaceDiffBaselineUnavailable(EmptyErrorParams {})
        );
    }

    /// Opens one isolated backend whose worktrees stay inside the test fixture.
    fn open_backend(temporary: &TempDir) -> Backend {
        Backend::open(BackendPaths {
            database_path: temporary.path().join("ora.sqlite3"),
            data_directory: temporary.path().to_path_buf(),
            deno_path: std::path::PathBuf::from("deno"),
            worktree_root: temporary.path().join("worktrees"),
            home_directory: temporary.path().to_path_buf(),
            relative_path_base: temporary.path().to_path_buf(),
            sessions_root: temporary.path().join("sessions"),
            skills_root: temporary.path().join("atoms").join("skills"),
            ripgrep_path: std::path::PathBuf::from("rg"),
            timezone: chrono_tz::UTC,
        })
        .expect("open shared backend")
    }

    /// Persists a project pointing at the initialized repository fixture.
    fn create_project(backend: &Backend, repository_root: &Path) -> String {
        backend
            .create_project(CreateProjectRequest {
                name: "Ora".to_string(),
                main_workspace_path: repository_root.to_string_lossy().into_owned(),
            })
            .expect("create project")
            .project
            .id
    }
}
