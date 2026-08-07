use std::collections::HashMap;
use std::path::Path;

use gitlancer::git::branch::{CheckoutBranchRequest, CreateBranchRequest, ListBranchesRequest};
use gitlancer::git::commit::{
    AddRequest, CommitRequest, StageAllRequest, UnstageAllRequest, UnstageRequest,
};
use gitlancer::git::conflict::{ConflictSide, ResolveConflictRequest};
use gitlancer::git::diff::{CommitDiffRequest, DiffRequest, DiffScope};
use gitlancer::git::history::{
    CommitDetails, CommitSummary, GetCommitRequest, ListCommitsRequest, ListReferencesRequest,
    ReferenceKind,
};
use gitlancer::git::pull::FastForwardRequest;
use gitlancer::git::remote::{FetchAllRequest, ReadTrackingStatusRequest};
use gitlancer::git::repository::ListWorktreesRequest;
use gitlancer::git::status::{StatusRequest, StatusResponse};
use gitlancer::git::sync::{
    AbortSyncRequest, ContinueSyncRequest, IntegrateRequest, ReadSyncOperationRequest,
    SyncOperation as GitSyncOperation, SyncResult,
};
use gitlancer::{BranchName, CliGitRunner, Git, GitlancerError, RepoRoot, WorktreeKind};
use ora_application::ProjectRepository;
use ora_contracts::{
    CheckoutRepositoryBranchRequest, CheckoutRepositoryBranchResponse,
    CommitRepositoryChangesRequest, CommitRepositoryChangesResponse, CreateRepositoryBranchRequest,
    CreateRepositoryBranchResponse, FetchRepositoryRequest, FetchRepositoryResponse,
    GetRepositoryCommitDiffRequest, GetRepositoryCommitDiffResponse, GetRepositoryCommitRequest,
    GetRepositoryCommitResponse, GetRepositorySnapshotRequest, GetRepositorySnapshotResponse,
    GetRepositoryWorkingTreeDiffRequest, GetRepositoryWorkingTreeDiffResponse,
    PullRepositoryOutcome, PullRepositoryRequest, PullRepositoryResponse, PullRepositoryStrategy,
    PushRepositoryBranchRequest, PushRepositoryBranchResponse, RepositoryBranchParams,
    RepositoryChangeSelection, RepositoryCommit, RepositoryCommitDetails, RepositoryCommitFile,
    RepositoryConflictSide, RepositoryRefKind, RepositoryReference, RepositoryRemoteStatus,
    RepositorySnapshot, RepositorySyncAction, RepositorySyncOperation, RepositorySyncOutcome,
    RepositoryWorkingTree, RepositoryWorkingTreeDiff, RepositoryWorkingTreeFile,
    ResolveRepositoryConflictRequest, ResolveRepositoryConflictResponse,
    ResolveRepositorySyncRequest, ResolveRepositorySyncResponse, StageRepositoryChangesRequest,
    StageRepositoryChangesResponse, UnstageRepositoryChangesRequest,
    UnstageRepositoryChangesResponse,
};
use ora_db::{RepositoryPool, SqliteProjectRepository};
use ora_domain::ProjectId;

use crate::{BackendError, ErrorClassification};
use ora_contracts::{EmptyErrorParams, PublicError};

const HISTORY_LIMIT: usize = 200;

/// Converts a repository reader thread panic into a backend failure instead of panicking again.
fn join_repository_reader<'scope, T: Send + 'scope>(
    handle: std::thread::ScopedJoinHandle<'scope, T>,
    context: &'static str,
) -> Result<T, BackendError> {
    handle.join().map_err(|_| {
        BackendError::new(
            ErrorClassification::Internal,
            PublicError::InternalError(EmptyErrorParams {}),
            context,
        )
    })
}

/// Owns repository graph reads and guarded branch operations shared by Web and Tauri adapters.
pub(crate) struct RepositoryApi {
    pool: RepositoryPool,
}

impl RepositoryApi {
    /// Builds the repository graph API from the shared project storage pool.
    pub(crate) fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }

    /// Reads the bounded repository graph and current worktree status for one project.
    pub(crate) fn get_snapshot(
        &self,
        request: GetRepositorySnapshotRequest,
    ) -> Result<GetRepositorySnapshotResponse, BackendError> {
        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = git
            .discover_repository(RepoRoot::new(Path::new(&project.root_path)))
            .map_err(|source| BackendError::internal("repository discovery failed", source))?;
        let (head, references, commits, working_tree, remote_status, sync_operation) =
            std::thread::scope(|scope| {
                // These queries only read independent Git state, so running them together removes
                // their process startup time from the critical path one query at a time.
                let head = scope.spawn(|| git.read_head(&repository));
                let references = scope.spawn(|| {
                    git.list_references(ListReferencesRequest {
                        repository: &repository,
                    })
                });
                let commits = scope.spawn(|| {
                    git.list_commits(ListCommitsRequest {
                        repository: &repository,
                        limit: HISTORY_LIMIT,
                    })
                });
                let working_tree = scope.spawn(|| read_working_tree(&git, &repository));
                let remote_status = scope.spawn(|| read_remote_status(&git, &repository));
                let sync_operation = scope.spawn(|| read_sync_operation(&git, &repository));

                (
                    join_repository_reader(head, "repository head reader panicked"),
                    join_repository_reader(references, "repository ref reader panicked"),
                    join_repository_reader(commits, "repository history reader panicked"),
                    join_repository_reader(working_tree, "repository worktree reader panicked"),
                    join_repository_reader(remote_status, "repository remote reader panicked"),
                    join_repository_reader(sync_operation, "repository sync reader panicked"),
                )
            });
        let head = head?;
        let references = references?;
        let commits = commits?;
        let working_tree = working_tree?;
        let remote_status = remote_status?;
        let sync_operation = sync_operation?;
        let head =
            head.map_err(|source| BackendError::internal("repository head lookup failed", source))?;
        let references = references
            .map_err(|source| BackendError::internal("repository ref lookup failed", source))?;
        let commits = commits
            .map_err(|source| BackendError::internal("repository history lookup failed", source))?;
        let working_tree = working_tree?;
        let remote_status = remote_status?;
        let sync_operation = sync_operation?;
        let reference_names_by_commit = reference_names_by_commit(&references.references);

        Ok(GetRepositorySnapshotResponse {
            snapshot: RepositorySnapshot {
                project_id: request.project_id,
                root_path: repository.root().as_path().to_string_lossy().into_owned(),
                head_commit_id: head
                    .commit_id
                    .map(|commit_id| commit_id.as_str().to_string()),
                current_branch: head
                    .branch_name
                    .map(|branch_name| branch_name.as_str().to_string()),
                references: references
                    .references
                    .into_iter()
                    .map(map_reference)
                    .collect(),
                commits: commits
                    .commits
                    .iter()
                    .map(|commit| map_commit(commit, &reference_names_by_commit))
                    .collect(),
                working_tree,
                remote_status,
                sync_operation,
            },
        })
    }

    /// Reads one commit's metadata and changed paths without loading its potentially large patch.
    pub(crate) fn get_commit(
        &self,
        request: GetRepositoryCommitRequest,
    ) -> Result<GetRepositoryCommitResponse, BackendError> {
        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = git
            .discover_repository(RepoRoot::new(Path::new(&project.root_path)))
            .map_err(|source| BackendError::internal("repository discovery failed", source))?;
        let commit_id = gitlancer::CommitId::new(request.commit_id);
        let response = git
            .get_commit(GetCommitRequest {
                repository: &repository,
                commit_id: &commit_id,
            })
            .map_err(|source| BackendError::internal("repository commit lookup failed", source))?;
        Ok(GetRepositoryCommitResponse {
            commit: map_commit_details(&response.commit),
        })
    }

    /// Reads one historical commit patch only after the UI opens a specific file.
    pub(crate) fn get_commit_diff(
        &self,
        request: GetRepositoryCommitDiffRequest,
    ) -> Result<GetRepositoryCommitDiffResponse, BackendError> {
        if request.path.trim().is_empty() {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "repository commit diff path must not be blank",
            ));
        }
        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = git
            .discover_repository(RepoRoot::new(Path::new(&project.root_path)))
            .map_err(|source| BackendError::internal("repository discovery failed", source))?;
        let commit_id = gitlancer::CommitId::new(request.commit_id);
        let parent_commit_id = request.parent_commit_id.map(gitlancer::CommitId::new);
        let diff = git
            .diff_commit(CommitDiffRequest {
                repository: &repository,
                commit_id: &commit_id,
                parent_commit_id: parent_commit_id.as_ref(),
                path: Some(&request.path),
            })
            .map_err(|source| {
                BackendError::internal("repository commit diff lookup failed", source)
            })?;

        Ok(GetRepositoryCommitDiffResponse { patch: diff.patch })
    }

    /// Reads the current main checkout patch without requiring an Ora task worktree.
    pub(crate) fn get_working_tree_diff(
        &self,
        request: GetRepositoryWorkingTreeDiffRequest,
    ) -> Result<GetRepositoryWorkingTreeDiffResponse, BackendError> {
        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = git
            .discover_repository(RepoRoot::new(Path::new(&project.root_path)))
            .map_err(|source| BackendError::internal("repository discovery failed", source))?;
        let head = git
            .read_head(&repository)
            .map_err(|source| BackendError::internal("repository head lookup failed", source))?;

        let Some(head_commit_id) = head.commit_id else {
            return Ok(GetRepositoryWorkingTreeDiffResponse {
                diff: RepositoryWorkingTreeDiff {
                    head_commit_id: None,
                    patch: String::new(),
                },
            });
        };
        let main_worktree = main_worktree(&git, &repository)?;
        let diff = git
            .diff(DiffRequest {
                worktree: &main_worktree,
                base_commit_id: &head_commit_id,
                scope: DiffScope::Branch,
            })
            .map_err(|source| {
                BackendError::internal("repository working tree diff lookup failed", source)
            })?;

        Ok(GetRepositoryWorkingTreeDiffResponse {
            diff: RepositoryWorkingTreeDiff {
                head_commit_id: Some(diff.head_commit_id.as_str().to_string()),
                patch: diff.patch,
            },
        })
    }

    /// Creates a local branch from the repository's current HEAD without changing the checkout.
    pub(crate) fn create_branch(
        &self,
        request: CreateRepositoryBranchRequest,
    ) -> Result<CreateRepositoryBranchResponse, BackendError> {
        let branch_name = validated_branch_name(&request.branch_name)?;
        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = discover_repository(&git, &project.root_path)?;
        let head = git
            .read_head(&repository)
            .map_err(|source| BackendError::internal("repository head lookup failed", source))?;
        let commit_id = head.commit_id.ok_or_else(|| {
            BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "cannot create a branch before the repository has a commit",
            )
        })?;
        let response = git
            .create_branch(CreateBranchRequest {
                repository: &repository,
                branch_name,
                commit_id,
            })
            .map_err(|source| map_branch_error(source, "repository branch creation failed"))?;

        Ok(CreateRepositoryBranchResponse {
            branch: response.branch.as_str().to_string(),
        })
    }

    /// Checks out an existing branch only after confirming the main worktree is clean.
    pub(crate) fn checkout_branch(
        &self,
        request: CheckoutRepositoryBranchRequest,
    ) -> Result<CheckoutRepositoryBranchResponse, BackendError> {
        let branch_name = validated_branch_name(&request.branch_name)?;
        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = discover_repository(&git, &project.root_path)?;
        let branches = git
            .list_branches(ListBranchesRequest {
                repository: &repository,
            })
            .map_err(|source| BackendError::internal("repository branch lookup failed", source))?;
        if !branches
            .branches
            .iter()
            .any(|branch| branch == &branch_name)
        {
            return Err(repository_branch_not_found(&branch_name));
        }

        let main_worktree = main_worktree(&git, &repository)?;
        let status = git
            .status(StatusRequest {
                worktree: &main_worktree,
            })
            .map_err(|source| BackendError::internal("repository status lookup failed", source))?;
        if !status.entries.is_empty() {
            return Err(BackendError::new(
                ErrorClassification::Conflict,
                PublicError::RepositoryWorktreeDirty(EmptyErrorParams {}),
                "cannot switch branches while the main worktree has uncommitted changes",
            ));
        }

        let response = git
            .checkout_branch(CheckoutBranchRequest {
                worktree: &main_worktree,
                branch_name: &branch_name,
            })
            .map_err(|source| map_branch_error(source, "repository branch checkout failed"))?;

        Ok(CheckoutRepositoryBranchResponse {
            branch: response.branch.as_str().to_string(),
        })
    }

    /// Fetches all configured remotes and returns the refreshed repository snapshot.
    pub(crate) fn fetch(
        &self,
        request: FetchRepositoryRequest,
    ) -> Result<FetchRepositoryResponse, BackendError> {
        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = discover_repository(&git, &project.root_path)?;
        git.fetch_all(FetchAllRequest {
            repository: &repository,
        })
        .map_err(|source| BackendError::internal("repository fetch failed", source))?;

        Ok(FetchRepositoryResponse {
            snapshot: self
                .get_snapshot(GetRepositorySnapshotRequest {
                    project_id: request.project_id,
                })?
                .snapshot,
        })
    }

    /// Fetches the upstream and applies the caller's explicit synchronization strategy.
    pub(crate) fn pull(
        &self,
        request: PullRepositoryRequest,
    ) -> Result<PullRepositoryResponse, BackendError> {
        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = discover_repository(&git, &project.root_path)?;
        let worktree = main_worktree(&git, &repository)?;
        if let Some(operation) = read_sync_operation(&git, &repository)? {
            return Ok(PullRepositoryResponse {
                outcome: PullRepositoryOutcome::Conflicted { operation },
                snapshot: self
                    .get_snapshot(GetRepositorySnapshotRequest {
                        project_id: request.project_id,
                    })?
                    .snapshot,
            });
        }

        let working_tree = read_working_tree(&git, &repository)?;
        if working_tree.changed_files > 0 {
            return Err(BackendError::new(
                ErrorClassification::Conflict,
                PublicError::RepositoryWorktreeDirty(EmptyErrorParams {}),
                "cannot pull while the main worktree has uncommitted changes",
            ));
        }

        git.fetch_all(FetchAllRequest {
            repository: &repository,
        })
        .map_err(|source| BackendError::internal("repository pull fetch failed", source))?;

        let tracking = git
            .read_tracking_status(ReadTrackingStatusRequest {
                repository: &repository,
            })
            .map_err(|source| {
                BackendError::internal("repository pull tracking lookup failed", source)
            })?;
        let Some(upstream) = tracking.upstream else {
            return Err(BackendError::new(
                ErrorClassification::Conflict,
                PublicError::RepositoryUpstreamNotConfigured(EmptyErrorParams {}),
                "cannot pull without a configured upstream branch",
            ));
        };

        let outcome = match (tracking.ahead, tracking.behind, request.strategy) {
            (_, 0, _) => PullRepositoryOutcome::AlreadyUpToDate,
            (0, _, _) => {
                git.fast_forward(FastForwardRequest {
                    worktree: &worktree,
                    upstream: &upstream,
                })
                .map_err(|source| {
                    BackendError::internal("repository fast-forward failed", source)
                })?;
                PullRepositoryOutcome::FastForwarded
            }
            (ahead, behind, PullRepositoryStrategy::FastForwardOnly) => {
                PullRepositoryOutcome::Diverged { ahead, behind }
            }
            (_, _, strategy) => {
                let (operation, completed) = match strategy {
                    PullRepositoryStrategy::Merge => {
                        (GitSyncOperation::Merge, PullRepositoryOutcome::Merged)
                    }
                    PullRepositoryStrategy::Rebase => {
                        (GitSyncOperation::Rebase, PullRepositoryOutcome::Rebased)
                    }
                    PullRepositoryStrategy::FastForwardOnly => unreachable!(),
                };
                match git
                    .integrate(IntegrateRequest {
                        repository: &repository,
                        worktree: &worktree,
                        upstream: &upstream,
                        operation,
                    })
                    .map_err(|source| {
                        BackendError::internal("repository integration failed", source)
                    })? {
                    SyncResult::Completed => completed,
                    SyncResult::Conflicted => PullRepositoryOutcome::Conflicted {
                        operation: map_sync_operation(operation),
                    },
                }
            }
        };

        Ok(PullRepositoryResponse {
            outcome,
            snapshot: self
                .get_snapshot(GetRepositorySnapshotRequest {
                    project_id: request.project_id,
                })?
                .snapshot,
        })
    }

    /// Continues or aborts the active main-worktree merge/rebase operation.
    pub(crate) fn resolve_sync(
        &self,
        request: ResolveRepositorySyncRequest,
    ) -> Result<ResolveRepositorySyncResponse, BackendError> {
        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = discover_repository(&git, &project.root_path)?;
        let worktree = main_worktree(&git, &repository)?;
        let operation = git
            .read_sync_operation(ReadSyncOperationRequest {
                repository: &repository,
            })
            .map_err(|source| {
                BackendError::internal("repository synchronization lookup failed", source)
            })?
            .ok_or_else(|| {
                BackendError::new(
                    ErrorClassification::Conflict,
                    PublicError::RepositorySyncNotInProgress(EmptyErrorParams {}),
                    "there is no active merge or rebase to resolve",
                )
            })?;

        let outcome = match request.action {
            RepositorySyncAction::Abort => {
                git.abort_sync(AbortSyncRequest {
                    repository: &repository,
                    worktree: &worktree,
                    operation,
                })
                .map_err(|source| {
                    BackendError::internal("repository synchronization abort failed", source)
                })?;
                RepositorySyncOutcome::Aborted
            }
            RepositorySyncAction::Continue => match git
                .continue_sync(ContinueSyncRequest {
                    repository: &repository,
                    worktree: &worktree,
                    operation,
                })
                .map_err(|source| {
                    BackendError::internal("repository synchronization continuation failed", source)
                })? {
                SyncResult::Completed => RepositorySyncOutcome::Completed,
                SyncResult::Conflicted => RepositorySyncOutcome::Conflicted,
            },
        };

        Ok(ResolveRepositorySyncResponse {
            outcome,
            snapshot: self
                .get_snapshot(GetRepositorySnapshotRequest {
                    project_id: request.project_id,
                })?
                .snapshot,
        })
    }

    /// Selects and stages one side of a conflicted path in the main worktree.
    pub(crate) fn resolve_conflict(
        &self,
        request: ResolveRepositoryConflictRequest,
    ) -> Result<ResolveRepositoryConflictResponse, BackendError> {
        if request.path.trim().is_empty() {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "repository conflict path must not be blank",
            ));
        }

        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = discover_repository(&git, &project.root_path)?;
        let worktree = main_worktree(&git, &repository)?;
        let path = worktree
            .resolve_repo_relative_path(Path::new(&request.path))
            .map_err(|source| {
                BackendError::with_source(
                    ErrorClassification::InvalidRequest,
                    PublicError::InvalidRequest(EmptyErrorParams {}),
                    "repository conflict path is invalid",
                    source,
                )
            })?;
        if path.as_path().as_os_str().is_empty() {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "repository conflict path must identify a file",
            ));
        }

        git.resolve_conflict(ResolveConflictRequest {
            worktree: &worktree,
            path: &path,
            side: map_conflict_side(request.side),
        })
        .map_err(|source| {
            BackendError::internal("repository conflict resolution failed", source)
        })?;

        Ok(ResolveRepositoryConflictResponse {
            working_tree: read_working_tree(&git, &repository)?,
        })
    }

    /// Pushes the checked-out main branch to origin and returns refreshed tracking metadata.
    pub(crate) fn push_branch(
        &self,
        request: PushRepositoryBranchRequest,
    ) -> Result<PushRepositoryBranchResponse, BackendError> {
        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = discover_repository(&git, &project.root_path)?;
        let worktree = main_worktree(&git, &repository)?;
        let response = git
            .push_branch(&worktree)
            .map_err(|source| BackendError::internal("repository push failed", source))?;

        Ok(PushRepositoryBranchResponse {
            branch_name: response.branch_name,
            remote_name: response.remote_name,
            snapshot: self
                .get_snapshot(GetRepositorySnapshotRequest {
                    project_id: request.project_id,
                })?
                .snapshot,
        })
    }

    /// Stages selected paths in the main checkout and returns the refreshed status summary.
    pub(crate) fn stage_changes(
        &self,
        request: StageRepositoryChangesRequest,
    ) -> Result<StageRepositoryChangesResponse, BackendError> {
        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = discover_repository(&git, &project.root_path)?;
        let worktree = main_worktree(&git, &repository)?;

        match request.selection {
            RepositoryChangeSelection::All => {
                git.stage_all(StageAllRequest {
                    worktree: &worktree,
                })
                .map_err(|source| BackendError::internal("repository staging failed", source))?;
            }
            RepositoryChangeSelection::Paths(paths) => {
                let paths = resolve_change_paths(&worktree, paths)?;
                git.add(AddRequest {
                    worktree: &worktree,
                    paths,
                })
                .map_err(|source| BackendError::internal("repository staging failed", source))?;
            }
        }

        Ok(StageRepositoryChangesResponse {
            working_tree: read_working_tree(&git, &repository)?,
        })
    }

    /// Removes selected paths from the index in the main checkout and returns refreshed status.
    pub(crate) fn unstage_changes(
        &self,
        request: UnstageRepositoryChangesRequest,
    ) -> Result<UnstageRepositoryChangesResponse, BackendError> {
        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = discover_repository(&git, &project.root_path)?;
        let worktree = main_worktree(&git, &repository)?;

        match request.selection {
            RepositoryChangeSelection::All => {
                git.unstage_all(UnstageAllRequest {
                    worktree: &worktree,
                })
                .map_err(|source| BackendError::internal("repository unstaging failed", source))?;
            }
            RepositoryChangeSelection::Paths(paths) => {
                let paths = resolve_change_paths(&worktree, paths)?;
                git.unstage(UnstageRequest {
                    worktree: &worktree,
                    paths,
                })
                .map_err(|source| BackendError::internal("repository unstaging failed", source))?;
            }
        }

        Ok(UnstageRepositoryChangesResponse {
            working_tree: read_working_tree(&git, &repository)?,
        })
    }

    /// Commits the currently staged main-checkout changes and returns the new commit metadata.
    pub(crate) fn commit_changes(
        &self,
        request: CommitRepositoryChangesRequest,
    ) -> Result<CommitRepositoryChangesResponse, BackendError> {
        let message = request.message.trim();
        if message.is_empty() {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::RepositoryCommitMessageBlank(EmptyErrorParams {}),
                "repository commit message must not be blank",
            ));
        }

        let project = self.load_project(&request.project_id)?;
        let git = Git::new(CliGitRunner);
        let repository = discover_repository(&git, &project.root_path)?;
        let worktree = main_worktree(&git, &repository)?;
        let working_tree = read_working_tree(&git, &repository)?;
        if working_tree.staged_files == 0 {
            return Err(BackendError::new(
                ErrorClassification::Conflict,
                PublicError::RepositoryNothingStaged(EmptyErrorParams {}),
                "repository has no staged changes to commit",
            ));
        }

        let response = git
            .commit(CommitRequest {
                worktree: &worktree,
                message,
                allow_empty: false,
            })
            .map_err(|source| BackendError::internal("repository commit failed", source))?;

        Ok(CommitRepositoryChangesResponse {
            commit_id: response.commit_id.as_str().to_string(),
            summary: response.summary,
            working_tree: read_working_tree(&git, &repository)?,
        })
    }

    /// Loads one visible project while preserving the stable project-not-found contract.
    fn load_project(&self, project_id: &str) -> Result<ora_domain::Project, BackendError> {
        SqliteProjectRepository::new(self.pool.clone())
            .find_project(&ProjectId::new(project_id))
            .map_err(|source| BackendError::internal("repository project lookup failed", source))?
            .ok_or_else(|| {
                BackendError::new(
                    ErrorClassification::NotFound,
                    PublicError::ProjectNotFound(EmptyErrorParams {}),
                    format!("project not found: {project_id}"),
                )
            })
    }
}

/// Loads a Git repository from a persisted project root while keeping adapter errors uniform.
fn discover_repository(
    git: &Git<CliGitRunner>,
    root_path: &str,
) -> Result<gitlancer::Repository, BackendError> {
    git.discover_repository(RepoRoot::new(Path::new(root_path)))
        .map_err(|source| BackendError::internal("repository discovery failed", source))
}

/// Rejects blank branch names before they reach Git so the public failure is actionable.
fn validated_branch_name(branch_name: &str) -> Result<BranchName, BackendError> {
    let trimmed = branch_name.trim();
    if trimmed.is_empty() {
        return Err(BackendError::new(
            ErrorClassification::InvalidRequest,
            PublicError::RepositoryBranchNameBlank(EmptyErrorParams {}),
            "repository branch name must not be blank",
        ));
    }

    Ok(BranchName::new(trimmed.to_string()))
}

/// Maps branch-domain failures into stable public errors while keeping Git diagnostics private.
fn map_branch_error(source: GitlancerError, context: &'static str) -> BackendError {
    match source {
        GitlancerError::Domain(gitlancer::DomainError::BranchAlreadyExists { branch, .. }) => {
            BackendError::new(
                ErrorClassification::Conflict,
                PublicError::RepositoryBranchAlreadyExists(RepositoryBranchParams {
                    branch_name: branch,
                }),
                "repository branch already exists",
            )
        }
        GitlancerError::Domain(gitlancer::DomainError::BranchNotFound { branch, .. }) => {
            repository_branch_not_found(&BranchName::new(branch))
        }
        other => BackendError::internal_boxed(context, Box::new(other)),
    }
}

/// Builds the typed not-found failure shared by branch existence checks and Git domain errors.
fn repository_branch_not_found(branch_name: &BranchName) -> BackendError {
    BackendError::new(
        ErrorClassification::NotFound,
        PublicError::RepositoryBranchNotFound(RepositoryBranchParams {
            branch_name: branch_name.as_str().to_string(),
        }),
        "repository branch was not found",
    )
}

/// Reads porcelain status from the main checkout because the graph describes repository state, not an Ora task.
fn read_working_tree(
    git: &Git<CliGitRunner>,
    repository: &gitlancer::Repository,
) -> Result<RepositoryWorkingTree, BackendError> {
    let main_worktree = main_worktree(git, repository)?;
    let status = git
        .status(StatusRequest {
            worktree: &main_worktree,
        })
        .map_err(|source| BackendError::internal("repository status lookup failed", source))?;

    Ok(summarize_working_tree(&status))
}

/// Reads the current upstream relationship without exposing raw Git plumbing output to adapters.
fn read_remote_status(
    git: &Git<CliGitRunner>,
    repository: &gitlancer::Repository,
) -> Result<RepositoryRemoteStatus, BackendError> {
    let status = git
        .read_tracking_status(ReadTrackingStatusRequest { repository })
        .map_err(|source| {
            BackendError::internal("repository tracking status lookup failed", source)
        })?;

    Ok(RepositoryRemoteStatus {
        upstream: status.upstream,
        ahead: status.ahead,
        behind: status.behind,
    })
}

/// Reads the active merge/rebase operation so the graph can expose an actionable sync state.
fn read_sync_operation(
    git: &Git<CliGitRunner>,
    repository: &gitlancer::Repository,
) -> Result<Option<RepositorySyncOperation>, BackendError> {
    git.read_sync_operation(ReadSyncOperationRequest { repository })
        .map(|operation| operation.map(map_sync_operation))
        .map_err(|source| BackendError::internal("repository sync state lookup failed", source))
}

/// Converts the Git runtime operation into the shared transport enum.
fn map_sync_operation(operation: GitSyncOperation) -> RepositorySyncOperation {
    match operation {
        GitSyncOperation::Merge => RepositorySyncOperation::Merge,
        GitSyncOperation::Rebase => RepositorySyncOperation::Rebase,
    }
}

/// Converts the shared conflict side into the Git runtime side enum.
fn map_conflict_side(side: RepositoryConflictSide) -> ConflictSide {
    match side {
        RepositoryConflictSide::Ours => ConflictSide::Ours,
        RepositoryConflictSide::Theirs => ConflictSide::Theirs,
    }
}

/// Resolves explicit UI paths through the worktree boundary before passing them to Git.
fn resolve_change_paths(
    worktree: &gitlancer::WorktreeHandle,
    paths: Vec<String>,
) -> Result<Vec<gitlancer::RepoRelativePath>, BackendError> {
    if paths.is_empty() {
        return Err(BackendError::new(
            ErrorClassification::InvalidRequest,
            PublicError::InvalidRequest(EmptyErrorParams {}),
            "repository change selection must contain at least one path",
        ));
    }

    paths
        .into_iter()
        .map(|path| {
            worktree
                .resolve_repo_relative_path(Path::new(&path))
                .map_err(|source| {
                    BackendError::with_source(
                        ErrorClassification::InvalidRequest,
                        PublicError::InvalidRequest(EmptyErrorParams {}),
                        "repository change path is invalid",
                        source,
                    )
                })
        })
        .collect()
}

/// Resolves the main checkout once so status and diff reads use the same repository context.
fn main_worktree(
    git: &Git<CliGitRunner>,
    repository: &gitlancer::Repository,
) -> Result<gitlancer::WorktreeHandle, BackendError> {
    let worktrees = git
        .list_worktrees(ListWorktreesRequest { repository })
        .map_err(|source| BackendError::internal("repository worktree lookup failed", source))?;
    worktrees
        .worktrees
        .into_iter()
        .find(|worktree| matches!(worktree.kind(), WorktreeKind::Main))
        .ok_or_else(|| {
            BackendError::internal(
                "repository main worktree is unavailable",
                std::io::Error::other("Git did not report a main worktree"),
            )
        })
}

/// Converts raw porcelain-v2 records into bounded counters suitable for a status summary.
fn summarize_working_tree(status: &StatusResponse) -> RepositoryWorkingTree {
    let mut summary = RepositoryWorkingTree {
        changed_files: 0,
        staged_files: 0,
        unstaged_files: 0,
        untracked_files: 0,
        conflicted_files: 0,
        files: Vec::new(),
    };

    for entry in &status.entries {
        let Some(file) = map_working_tree_file(&entry.raw) else {
            continue;
        };

        summary.changed_files += 1;
        if file.staged {
            summary.staged_files += 1;
        }
        if file.unstaged {
            summary.unstaged_files += 1;
        }
        if file.status == "??" {
            summary.untracked_files += 1;
        }
        if entry.raw.starts_with("u ") {
            summary.conflicted_files += 1;
        }
        summary.files.push(file);
    }

    summary
}

/// Converts one recognized porcelain-v2 record into the path metadata needed by Changes actions.
fn map_working_tree_file(raw: &str) -> Option<RepositoryWorkingTreeFile> {
    let (status, staged, unstaged, path) = if raw.starts_with("? ") {
        ("??", false, true, raw.get(2..)?)
    } else if raw.starts_with("! ") {
        return None;
    } else if raw.starts_with("u ") {
        let path = raw.splitn(11, ' ').nth(10)?;
        let status_bytes = raw.as_bytes();
        (
            raw.get(2..4)?,
            status_bytes.get(2).is_some_and(|status| *status != b'.'),
            status_bytes.get(3).is_some_and(|status| *status != b'.'),
            path,
        )
    } else if raw.starts_with("1 ") {
        let path = raw.splitn(9, ' ').nth(8)?;
        let status_bytes = raw.as_bytes();
        (
            raw.get(2..4)?,
            status_bytes.get(2).is_some_and(|status| *status != b'.'),
            status_bytes.get(3).is_some_and(|status| *status != b'.'),
            path,
        )
    } else if raw.starts_with("2 ") {
        let path = raw.splitn(10, ' ').nth(9)?.split('\t').next()?;
        let status_bytes = raw.as_bytes();
        (
            raw.get(2..4)?,
            status_bytes.get(2).is_some_and(|status| *status != b'.'),
            status_bytes.get(3).is_some_and(|status| *status != b'.'),
            path,
        )
    } else {
        return None;
    };

    Some(RepositoryWorkingTreeFile {
        path: path.to_string(),
        status: status.to_string(),
        staged,
        unstaged,
    })
}

/// Groups ref labels by target commit so the graph can decorate rows without recomputing topology.
fn reference_names_by_commit(
    references: &[gitlancer::git::history::RepositoryReference],
) -> HashMap<String, Vec<String>> {
    let mut names_by_commit = HashMap::new();
    for reference in references {
        names_by_commit
            .entry(reference.commit_id.as_str().to_string())
            .or_insert_with(Vec::new)
            .push(reference.name.clone());
    }
    names_by_commit
}

/// Maps one Git ref into the transport-neutral graph reference DTO.
fn map_reference(reference: gitlancer::git::history::RepositoryReference) -> RepositoryReference {
    RepositoryReference {
        name: reference.name,
        commit_id: reference.commit_id.as_str().to_string(),
        kind: match reference.kind {
            ReferenceKind::Local => RepositoryRefKind::Local,
            ReferenceKind::Remote => RepositoryRefKind::Remote,
            ReferenceKind::Tag => RepositoryRefKind::Tag,
        },
    }
}

/// Maps one Git commit summary and its ref labels into the app-facing graph row.
fn map_commit(
    commit: &CommitSummary,
    reference_names_by_commit: &HashMap<String, Vec<String>>,
) -> RepositoryCommit {
    let id = commit.id.as_str().to_string();
    RepositoryCommit {
        short_id: short_commit_id(&id),
        parents: commit
            .parents
            .iter()
            .map(|parent| parent.as_str().to_string())
            .collect(),
        subject: commit.subject.clone(),
        author_name: commit.author_name.clone(),
        author_email: commit.author_email.clone(),
        authored_at: commit.authored_at.clone(),
        reference_names: reference_names_by_commit
            .get(&id)
            .cloned()
            .unwrap_or_default(),
        id,
    }
}

/// Maps detailed Git metadata into the commit detail contract without eagerly loading patch text.
fn map_commit_details(commit: &CommitDetails) -> RepositoryCommitDetails {
    let id = commit.summary.id.as_str().to_string();
    RepositoryCommitDetails {
        short_id: short_commit_id(&id),
        parents: commit
            .summary
            .parents
            .iter()
            .map(|parent| parent.as_str().to_string())
            .collect(),
        subject: commit.summary.subject.clone(),
        author_name: commit.summary.author_name.clone(),
        author_email: commit.summary.author_email.clone(),
        authored_at: commit.summary.authored_at.clone(),
        files: commit
            .files
            .iter()
            .map(|file| RepositoryCommitFile {
                status: file.status.clone(),
                path: file.path.clone(),
            })
            .collect(),
        id,
    }
}

/// Keeps the full object id available while using a compact, stable graph label.
fn short_commit_id(commit_id: &str) -> String {
    commit_id.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::summarize_working_tree;
    use crate::bootstrap::{Backend, BackendPaths};
    use crate::error::ErrorClassification;
    use gitlancer::git::status::{StatusEntry, StatusResponse};
    use ora_contracts::{
        CheckoutRepositoryBranchRequest, CommitRepositoryChangesRequest, CreateProjectRequest,
        CreateRepositoryBranchRequest, GetRepositoryCommitDiffRequest, GetRepositoryCommitRequest,
        GetRepositorySnapshotRequest, GetRepositoryWorkingTreeDiffRequest,
        RepositoryChangeSelection, RepositoryWorkingTree, RepositoryWorkingTreeFile,
        StageRepositoryChangesRequest, UnstageRepositoryChangesRequest,
    };
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    /// Verifies staged, unstaged, untracked, and conflicted porcelain records remain distinguishable.
    #[test]
    fn summarizes_porcelain_status() {
        let status = StatusResponse {
            entries: vec![
                StatusEntry {
                    raw: "1 M. N... 100644 100644 100644 abc def README.md".to_string(),
                },
                StatusEntry {
                    raw: "1 .M N... 100644 100644 100644 abc def src/lib.rs".to_string(),
                },
                StatusEntry {
                    raw: "? notes.txt".to_string(),
                },
                StatusEntry {
                    raw: "u UU N... 100644 100644 100644 100644 abc def ghi conflict.rs"
                        .to_string(),
                },
            ],
        };

        assert_eq!(
            summarize_working_tree(&status),
            RepositoryWorkingTree {
                changed_files: 4,
                staged_files: 2,
                unstaged_files: 3,
                untracked_files: 1,
                conflicted_files: 1,
                files: vec![
                    RepositoryWorkingTreeFile {
                        path: "README.md".to_string(),
                        status: "M.".to_string(),
                        staged: true,
                        unstaged: false,
                    },
                    RepositoryWorkingTreeFile {
                        path: "src/lib.rs".to_string(),
                        status: ".M".to_string(),
                        staged: false,
                        unstaged: true,
                    },
                    RepositoryWorkingTreeFile {
                        path: "notes.txt".to_string(),
                        status: "??".to_string(),
                        staged: false,
                        unstaged: true,
                    },
                    RepositoryWorkingTreeFile {
                        path: "conflict.rs".to_string(),
                        status: "UU".to_string(),
                        staged: true,
                        unstaged: true,
                    },
                ],
            }
        );
    }

    /// Verifies the shared backend can read a real repository snapshot and selected commit detail.
    #[test]
    fn reads_real_repository_graph_data() {
        let temporary = TempDir::new().expect("create repository fixture");
        let repository_root = temporary.path().join("repository");
        initialize_repository(&repository_root);
        let backend = Backend::open(BackendPaths {
            database_path: temporary.path().join("ora.sqlite3"),
            worktree_root: temporary.path().join("worktrees"),
            home_directory: temporary.path().to_path_buf(),
            sessions_root: temporary.path().join("sessions"),
            skills_root: temporary.path().join("atoms").join("skills"),
            ripgrep_path: std::path::PathBuf::from("rg"),
        })
        .expect("open backend");
        let project = backend
            .create_project(CreateProjectRequest {
                name: "Graph".to_string(),
                root_path: repository_root.to_string_lossy().into_owned(),
            })
            .expect("create project")
            .project;

        let project_id = project.id.clone();
        let snapshot = backend
            .get_repository_snapshot(GetRepositorySnapshotRequest {
                project_id: project_id.clone(),
            })
            .expect("read repository snapshot")
            .snapshot;
        let commit = backend
            .get_repository_commit(GetRepositoryCommitRequest {
                project_id: project_id.clone(),
                commit_id: snapshot.commits[0].id.clone(),
            })
            .expect("read repository commit")
            .commit;

        assert_eq!(snapshot.current_branch, Some("main".to_string()));
        assert_eq!(snapshot.commits.len(), 1);
        assert_eq!(snapshot.references[0].name, "main");
        assert_eq!(snapshot.working_tree.changed_files, 0);
        assert_eq!(commit.subject, "initial");
        assert_eq!(commit.files[0].path, "README.md");
        let commit_diff = backend
            .get_repository_commit_diff(GetRepositoryCommitDiffRequest {
                project_id: project_id.clone(),
                commit_id: snapshot.commits[0].id.clone(),
                parent_commit_id: None,
                path: "README.md".to_string(),
            })
            .expect("read repository commit diff");
        assert!(
            commit_diff
                .patch
                .contains("diff --git a/README.md b/README.md")
        );

        std::fs::write(repository_root.join("README.md"), "changed graph test\n")
            .expect("write working tree change");
        std::fs::write(repository_root.join("notes.md"), "untracked graph note\n")
            .expect("write untracked working tree file");
        let working_tree_diff = backend
            .get_repository_working_tree_diff(GetRepositoryWorkingTreeDiffRequest { project_id })
            .expect("read main working tree diff")
            .diff;
        assert!(
            working_tree_diff
                .patch
                .contains("diff --git a/README.md b/README.md")
        );
        assert!(
            working_tree_diff
                .patch
                .contains("diff --git a/notes.md b/notes.md")
        );
    }

    /// Verifies the main-worktree stage, unstage, and commit loop refreshes status and history.
    #[test]
    fn stages_unstages_and_commits_main_worktree_changes() {
        let temporary = TempDir::new().expect("create repository fixture");
        let repository_root = temporary.path().join("repository");
        initialize_repository(&repository_root);
        let backend = Backend::open(BackendPaths {
            database_path: temporary.path().join("ora.sqlite3"),
            worktree_root: temporary.path().join("worktrees"),
            home_directory: temporary.path().to_path_buf(),
            sessions_root: temporary.path().join("sessions"),
            skills_root: temporary.path().join("atoms").join("skills"),
            ripgrep_path: std::path::PathBuf::from("rg"),
        })
        .expect("open backend");
        let project_id = backend
            .create_project(CreateProjectRequest {
                name: "Commit loop".to_string(),
                root_path: repository_root.to_string_lossy().into_owned(),
            })
            .expect("create project")
            .project
            .id;

        std::fs::write(repository_root.join("README.md"), "staged README\n")
            .expect("write tracked change");
        std::fs::write(repository_root.join("notes.md"), "staged note\n")
            .expect("write untracked change");

        let staged = backend
            .stage_repository_changes(StageRepositoryChangesRequest {
                project_id: project_id.clone(),
                selection: RepositoryChangeSelection::Paths(vec!["README.md".to_string()]),
            })
            .expect("stage selected change")
            .working_tree;
        assert_eq!(staged.staged_files, 1);
        assert_eq!(staged.unstaged_files, 1);
        assert_eq!(staged.files[0].path, "README.md");
        assert_eq!(staged.files[0].status, "M.");
        assert_eq!(staged.files[1].status, "??");

        let unstaged = backend
            .unstage_repository_changes(UnstageRepositoryChangesRequest {
                project_id: project_id.clone(),
                selection: RepositoryChangeSelection::All,
            })
            .expect("unstage all changes")
            .working_tree;
        assert_eq!(unstaged.staged_files, 0);
        assert_eq!(unstaged.unstaged_files, 2);

        let restaged = backend
            .stage_repository_changes(StageRepositoryChangesRequest {
                project_id: project_id.clone(),
                selection: RepositoryChangeSelection::All,
            })
            .expect("stage all changes")
            .working_tree;
        assert_eq!(restaged.staged_files, 2);
        assert_eq!(restaged.unstaged_files, 0);

        let committed = backend
            .commit_repository_changes(CommitRepositoryChangesRequest {
                project_id: project_id.clone(),
                message: "commit main changes".to_string(),
            })
            .expect("commit staged changes");
        assert_eq!(committed.summary, "commit main changes");
        assert_eq!(committed.commit_id.len(), 40);
        assert_eq!(committed.working_tree.changed_files, 0);

        let snapshot = backend
            .get_repository_snapshot(GetRepositorySnapshotRequest {
                project_id: project_id.clone(),
            })
            .expect("refresh repository snapshot")
            .snapshot;
        assert_eq!(snapshot.commits.len(), 2);
        assert_eq!(snapshot.working_tree.changed_files, 0);

        let error = backend
            .commit_repository_changes(CommitRepositoryChangesRequest {
                project_id,
                message: "nothing staged".to_string(),
            })
            .expect_err("a clean index should not create an empty commit");
        assert_eq!(error.classification(), ErrorClassification::Conflict);
        assert_eq!(error.public_error().code(), "repository_nothing_staged");
    }

    /// Verifies branch creation, clean checkout, and dirty-worktree protection through the backend facade.
    #[test]
    fn protects_uncommitted_main_worktree_changes_during_branch_checkout() {
        let temporary = TempDir::new().expect("create repository fixture");
        let repository_root = temporary.path().join("repository");
        initialize_repository(&repository_root);
        let backend = Backend::open(BackendPaths {
            database_path: temporary.path().join("ora.sqlite3"),
            worktree_root: temporary.path().join("worktrees"),
            home_directory: temporary.path().to_path_buf(),
            sessions_root: temporary.path().join("sessions"),
            skills_root: temporary.path().join("atoms").join("skills"),
            ripgrep_path: std::path::PathBuf::from("rg"),
        })
        .expect("open backend");
        let project_id = backend
            .create_project(CreateProjectRequest {
                name: "Branch operations".to_string(),
                root_path: repository_root.to_string_lossy().into_owned(),
            })
            .expect("create project")
            .project
            .id;

        assert_eq!(
            backend
                .create_repository_branch(CreateRepositoryBranchRequest {
                    project_id: project_id.clone(),
                    branch_name: "feature/repository-workspace".to_string(),
                })
                .expect("create repository branch")
                .branch,
            "feature/repository-workspace"
        );
        assert_eq!(
            backend
                .checkout_repository_branch(CheckoutRepositoryBranchRequest {
                    project_id: project_id.clone(),
                    branch_name: "feature/repository-workspace".to_string(),
                })
                .expect("checkout clean repository")
                .branch,
            "feature/repository-workspace"
        );

        std::fs::write(
            repository_root.join("README.md"),
            "uncommitted branch change\n",
        )
        .expect("write uncommitted change");
        let error = backend
            .checkout_repository_branch(CheckoutRepositoryBranchRequest {
                project_id: project_id.clone(),
                branch_name: "main".to_string(),
            })
            .expect_err("dirty worktree should block checkout");

        assert_eq!(error.classification(), ErrorClassification::Conflict);
        assert_eq!(error.public_error().code(), "repository_worktree_dirty");
        assert_eq!(
            run_git_output(&repository_root, &["branch", "--show-current"]),
            "feature/repository-workspace"
        );

        std::fs::write(repository_root.join("README.md"), "graph test\n")
            .expect("restore clean worktree");
        assert_eq!(
            backend
                .checkout_repository_branch(CheckoutRepositoryBranchRequest {
                    project_id,
                    branch_name: "main".to_string(),
                })
                .expect("checkout restored repository")
                .branch,
            "main"
        );
    }

    /// Initializes one repository with a root commit for backend graph integration coverage.
    fn initialize_repository(repository_root: &Path) {
        std::fs::create_dir_all(repository_root).expect("create repository root");
        run_git(repository_root, &["init", "--initial-branch=main"]);
        run_git(repository_root, &["config", "user.name", "Ora Tests"]);
        run_git(
            repository_root,
            &["config", "user.email", "ora-tests@example.com"],
        );
        std::fs::write(repository_root.join("README.md"), "graph test\n")
            .expect("write repository seed");
        run_git(repository_root, &["add", "README.md"]);
        run_git(repository_root, &["commit", "-m", "initial"]);
    }

    /// Runs one repository setup command and preserves its arguments in fixture failures.
    fn run_git(repository_root: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .current_dir(repository_root)
            .args(arguments)
            .status()
            .expect("start git");
        assert!(status.success(), "git {arguments:?} failed with {status}");
    }

    /// Returns trimmed Git output for backend branch lifecycle assertions.
    fn run_git_output(repository_root: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(repository_root)
            .args(arguments)
            .output()
            .expect("start git");
        assert!(output.status.success(), "git {arguments:?} failed");
        String::from_utf8(output.stdout)
            .expect("Git output should be UTF-8")
            .trim()
            .to_string()
    }
}
