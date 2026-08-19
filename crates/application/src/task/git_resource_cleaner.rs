use super::git_cleanup::{
    CleanupStage, GitCleanupError, RemoveTaskBranchRequest, RemoveTaskWorktreeRequest,
    ResourceRemoval, TaskGitResourceCleaner, WorktreeRemoval,
};
use gitlancer::git::branch::{BranchDeletionMode, DeleteBranchRequest};
use gitlancer::git::repository::ListWorktreesRequest;
use gitlancer::git::worktree::{
    DeleteWorktreeRequest, ResolveWorktreeByBranchRequest, WorktreeDeletionMode,
};
use gitlancer::{
    BranchName, CliGitRunner, DomainError, Git, GitlancerError, RepoRoot, Repository,
    WorktreeHandle, WorktreeKind,
};
use std::path::{Path, PathBuf};

/// Removes task-owned Git resources through the shared Git runtime.
///
/// Stateless by design: every request carries the persisted repository root, so
/// one cleaner instance serves jobs across arbitrary repositories.
#[derive(Clone, Debug)]
pub struct GitTaskResourceCleaner {
    git: Git<CliGitRunner>,
}

impl Default for GitTaskResourceCleaner {
    fn default() -> Self {
        Self::new()
    }
}

impl GitTaskResourceCleaner {
    pub fn new() -> Self {
        Self {
            git: Git::new(CliGitRunner),
        }
    }

    /// Rediscovers the repository at execution time so a moved or replaced path
    /// fails the job instead of mutating whatever now lives there.
    fn discover(
        &self,
        repository_root: &Path,
        stage: CleanupStage,
    ) -> Result<Repository, GitCleanupError> {
        self.git
            .discover_repository(RepoRoot::new(repository_root))
            .map_err(|source| {
                GitCleanupError::new(stage, "failed to discover cleanup repository", source)
            })
    }

    /// Force-removes one resolved linked worktree, translating ownership guards.
    fn delete_resolved_worktree(
        &self,
        repository: &Repository,
        worktree: &WorktreeHandle,
    ) -> Result<WorktreeRemoval, GitCleanupError> {
        match self.git.delete_worktree(DeleteWorktreeRequest {
            repository,
            worktree,
            mode: WorktreeDeletionMode::Force,
        }) {
            // Git reporting success does not guarantee the directory is gone
            // (Windows removals can deregister the worktree yet leave the
            // folder behind), so removal is only confirmed on disk.
            Ok(_) => finish_checkout_removal(worktree.worktree_root().as_path()),
            // The main checkout and cross-repository worktrees are resources Ora
            // provably does not own; refusing them is a terminal safety outcome,
            // not a retryable failure.
            Err(GitlancerError::Domain(
                DomainError::MainWorktreeDeletionUnsupported(_)
                | DomainError::WorktreeMismatch { .. },
            )) => Ok(WorktreeRemoval::OwnershipLost),
            Err(source) => Err(GitCleanupError::new(
                CleanupStage::Worktree,
                "failed to remove task worktree",
                source,
            )),
        }
    }

    /// Resolves the exact recorded checkout among the repository's linked worktrees.
    ///
    /// Only an exact normalized root match counts: a prefix or containing match
    /// would resolve to a different (possibly main) checkout and violate the
    /// ownership evidence contract.
    fn find_linked_worktree_by_root(
        &self,
        repository: &Repository,
        checkout_root: &Path,
    ) -> Result<Option<WorktreeHandle>, GitCleanupError> {
        let worktrees = self
            .git
            .list_worktrees(ListWorktreesRequest { repository })
            .map_err(|source| {
                GitCleanupError::new(
                    CleanupStage::Worktree,
                    "failed to list repository worktrees",
                    source,
                )
            })?
            .worktrees;
        let target = normalize_root(checkout_root);
        Ok(worktrees.into_iter().find(|worktree| {
            matches!(worktree.kind(), WorktreeKind::Linked { .. })
                && normalize_root(worktree.worktree_root().as_path()) == target
        }))
    }
}

impl TaskGitResourceCleaner for GitTaskResourceCleaner {
    /// Resolves the worktree by branch first, then by recorded checkout root,
    /// and only reports `AlreadyAbsent` after positively confirming absence.
    fn remove_worktree(
        &self,
        request: RemoveTaskWorktreeRequest,
    ) -> Result<WorktreeRemoval, GitCleanupError> {
        let repository = self.discover(&request.repository_root, CleanupStage::Worktree)?;

        match self
            .git
            .resolve_worktree_by_branch(ResolveWorktreeByBranchRequest {
                repository: &repository,
                branch_name: &request.branch_name,
            }) {
            Ok(worktree) => return self.delete_resolved_worktree(&repository, &worktree),
            // No worktree is attached to the branch; fall through to the
            // recorded-checkout resolution for detached worktrees.
            Err(GitlancerError::Domain(DomainError::NotAWorktree(_))) => {}
            Err(source) => {
                return Err(GitCleanupError::new(
                    CleanupStage::Worktree,
                    "failed to resolve task worktree by branch",
                    source,
                ));
            }
        }

        let probe = request
            .checkout_root
            .as_deref()
            .or(request.legacy_checkout_probe.as_deref());
        let Some(probe) = probe else {
            // No recorded path exists to check; the branch has no worktree and
            // nothing else can be probed, so absence is confirmed.
            return Ok(WorktreeRemoval::AlreadyAbsent);
        };

        if let Some(worktree) = self.find_linked_worktree_by_root(&repository, probe)? {
            return self.delete_resolved_worktree(&repository, &worktree);
        }
        // Git no longer claims the recorded path. An empty directory there is a
        // half-failed removal's leftover shell — it holds no user data, so it is
        // reclaimed like any Ora residue. Only content Git cannot vouch for and
        // that actually contains something loses ownership.
        match probe_leftover(probe).map_err(|source| {
            GitCleanupError::new(
                CleanupStage::Worktree,
                "failed to inspect leftover checkout path",
                source,
            )
        })? {
            ProbeLeftover::Absent => Ok(WorktreeRemoval::AlreadyAbsent),
            ProbeLeftover::EmptyDirectory => {
                std::fs::remove_dir(probe).map_err(|source| {
                    GitCleanupError::new(
                        CleanupStage::Worktree,
                        "failed to remove leftover empty checkout directory",
                        source,
                    )
                })?;
                Ok(WorktreeRemoval::Removed)
            }
            ProbeLeftover::Occupied => Ok(WorktreeRemoval::OwnershipLost),
        }
    }

    /// Force-deletes the local branch; a missing branch is confirmed absence.
    fn remove_branch(
        &self,
        request: RemoveTaskBranchRequest,
    ) -> Result<ResourceRemoval, GitCleanupError> {
        let repository = self.discover(&request.repository_root, CleanupStage::Branch)?;
        match self.git.delete_branch(DeleteBranchRequest {
            repository: &repository,
            branch_name: BranchName::new(&request.branch_name),
            mode: BranchDeletionMode::Force,
        }) {
            Ok(_) => Ok(ResourceRemoval::Removed),
            Err(GitlancerError::Domain(DomainError::BranchNotFound { .. })) => {
                Ok(ResourceRemoval::AlreadyAbsent)
            }
            Err(source) => Err(GitCleanupError::new(
                CleanupStage::Branch,
                "failed to delete task branch",
                source,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{GitTaskResourceCleaner, finish_checkout_removal};
    use crate::task::git_cleanup::{
        RemoveTaskBranchRequest, RemoveTaskWorktreeRequest, ResourceRemoval,
        TaskGitResourceCleaner, WorktreeRemoval,
    };
    use ora_test_support::GitTestScaffold;
    use pretty_assertions::assert_eq;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Builds a real repository with one committed file and one linked task worktree.
    fn repository_with_task_worktree() -> (GitTestScaffold, PathBuf, PathBuf) {
        let scaffold =
            GitTestScaffold::new("git-resource-cleaner").expect("create Git test scaffold");
        scaffold
            .write_file(scaffold.repo_path(), "README.md", "fixture")
            .expect("write file");
        scaffold
            .stage_all_and_commit("init")
            .expect("create initial commit");
        let checkout = scaffold
            .create_linked_worktree("task-1", "ora/12345678")
            .expect("create linked worktree");
        let repo = scaffold.repo_path().to_path_buf();
        (scaffold, repo, checkout)
    }

    /// Verifies the branch-resolved removal path and its idempotent replay.
    #[test]
    fn removes_worktree_and_branch_then_confirms_absence_on_replay() {
        let (_scaffold, repo, checkout) = repository_with_task_worktree();
        let cleaner = GitTaskResourceCleaner::new();
        let worktree_request = RemoveTaskWorktreeRequest {
            repository_root: repo.clone(),
            branch_name: "ora/12345678".to_string(),
            checkout_root: Some(checkout.clone()),
            legacy_checkout_probe: None,
        };
        let branch_request = RemoveTaskBranchRequest {
            repository_root: repo.clone(),
            branch_name: "ora/12345678".to_string(),
        };

        assert_eq!(
            cleaner.remove_worktree(worktree_request.clone()).unwrap(),
            WorktreeRemoval::Removed
        );
        assert_eq!(
            cleaner.remove_branch(branch_request.clone()).unwrap(),
            ResourceRemoval::Removed
        );
        assert!(!checkout.exists());

        // Replay: both resources are now positively confirmed absent.
        assert_eq!(
            cleaner.remove_worktree(worktree_request).unwrap(),
            WorktreeRemoval::AlreadyAbsent
        );
        assert_eq!(
            cleaner.remove_branch(branch_request).unwrap(),
            ResourceRemoval::AlreadyAbsent
        );
    }

    /// Verifies a detached checkout is still resolved through its recorded root.
    #[test]
    fn removes_detached_worktree_through_recorded_checkout_root() {
        let (scaffold, repo, checkout) = repository_with_task_worktree();
        // Detach the worktree so branch-based resolution can no longer find it.
        scaffold
            .run_git_in(&checkout, ["checkout", "--detach"])
            .expect("detach worktree");
        let cleaner = GitTaskResourceCleaner::new();

        assert_eq!(
            cleaner
                .remove_worktree(RemoveTaskWorktreeRequest {
                    repository_root: repo.clone(),
                    branch_name: "ora/12345678".to_string(),
                    checkout_root: Some(checkout.clone()),
                    legacy_checkout_probe: None,
                })
                .unwrap(),
            WorktreeRemoval::Removed
        );
        assert!(!checkout.exists());
        assert_eq!(
            cleaner
                .remove_branch(RemoveTaskBranchRequest {
                    repository_root: repo,
                    branch_name: "ora/12345678".to_string(),
                })
                .unwrap(),
            ResourceRemoval::Removed
        );
    }

    /// Verifies dirty worktrees are still force-removed as the product decided.
    #[test]
    fn force_removes_dirty_worktrees() {
        let (scaffold, repo, checkout) = repository_with_task_worktree();
        scaffold
            .write_file(&checkout, "dirty.txt", "uncommitted")
            .expect("write dirty file");
        let cleaner = GitTaskResourceCleaner::new();

        assert_eq!(
            cleaner
                .remove_worktree(RemoveTaskWorktreeRequest {
                    repository_root: repo,
                    branch_name: "ora/12345678".to_string(),
                    checkout_root: Some(checkout.clone()),
                    legacy_checkout_probe: None,
                })
                .unwrap(),
            WorktreeRemoval::Removed
        );
        assert!(!checkout.exists());
    }

    /// Verifies a directory that is not a linked worktree is never removed.
    #[test]
    fn reports_ownership_lost_for_a_non_worktree_directory() {
        let (scaffold, repo, checkout) = repository_with_task_worktree();
        // Replace the checkout with a plain directory Git no longer tracks.
        scaffold
            .run_git([
                OsStr::new("worktree"),
                OsStr::new("remove"),
                checkout.as_os_str(),
                OsStr::new("--force"),
            ])
            .expect("remove linked worktree");
        scaffold
            .run_git(["branch", "-D", "ora/12345678"])
            .expect("remove linked branch");
        fs::create_dir_all(&checkout).expect("recreate plain directory");
        fs::write(checkout.join("user-data.txt"), "keep me").expect("write user data");
        let cleaner = GitTaskResourceCleaner::new();

        assert_eq!(
            cleaner
                .remove_worktree(RemoveTaskWorktreeRequest {
                    repository_root: repo.clone(),
                    branch_name: "ora/12345678".to_string(),
                    checkout_root: Some(checkout.clone()),
                    legacy_checkout_probe: None,
                })
                .unwrap(),
            WorktreeRemoval::OwnershipLost
        );
        assert!(checkout.join("user-data.txt").exists());

        // The main checkout can never be resolved as a removable linked worktree.
        assert_eq!(
            cleaner
                .remove_worktree(RemoveTaskWorktreeRequest {
                    repository_root: repo.clone(),
                    branch_name: "ora/99999999".to_string(),
                    checkout_root: Some(repo.clone()),
                    legacy_checkout_probe: None,
                })
                .unwrap(),
            WorktreeRemoval::OwnershipLost
        );
        assert!(repo.join("README.md").exists());
    }

    /// Verifies a half-failed removal's empty shell is reclaimed, not parked.
    ///
    /// This mirrors the Windows failure mode: `git worktree remove` deregisters
    /// the metadata but the directory survives; the retry must finish the job
    /// instead of misreading the empty shell as a user-owned checkout.
    #[test]
    fn reclaims_a_leftover_empty_shell_after_a_half_failed_removal() {
        let (scaffold, repo, checkout) = repository_with_task_worktree();
        scaffold
            .run_git([
                OsStr::new("worktree"),
                OsStr::new("remove"),
                checkout.as_os_str(),
                OsStr::new("--force"),
            ])
            .expect("remove linked worktree");
        // Recreate the shell a half-failed platform removal would leave behind.
        fs::create_dir_all(&checkout).expect("recreate empty shell");
        let cleaner = GitTaskResourceCleaner::new();

        assert_eq!(
            cleaner
                .remove_worktree(RemoveTaskWorktreeRequest {
                    repository_root: repo.clone(),
                    branch_name: "ora/12345678".to_string(),
                    checkout_root: None,
                    legacy_checkout_probe: Some(checkout.clone()),
                })
                .unwrap(),
            WorktreeRemoval::Removed
        );
        assert!(!checkout.exists());
        // The branch survives the worktree stage and is removed independently.
        assert_eq!(
            cleaner
                .remove_branch(RemoveTaskBranchRequest {
                    repository_root: repo,
                    branch_name: "ora/12345678".to_string(),
                })
                .unwrap(),
            ResourceRemoval::Removed
        );
    }

    /// Verifies post-removal confirmation across the three disk states.
    #[test]
    fn finish_checkout_removal_confirms_disk_state() {
        let temp = TempDir::new().expect("create temp dir");
        let absent = temp.path().join("absent");
        assert_eq!(
            finish_checkout_removal(&absent).unwrap(),
            WorktreeRemoval::Removed
        );

        let empty = temp.path().join("empty");
        fs::create_dir_all(&empty).expect("create empty dir");
        assert_eq!(
            finish_checkout_removal(&empty).unwrap(),
            WorktreeRemoval::Removed
        );
        assert!(!empty.exists());

        // Content Git failed to delete is a retryable failure, never silent success.
        let occupied = temp.path().join("occupied");
        fs::create_dir_all(&occupied).expect("create occupied dir");
        fs::write(occupied.join("locked.txt"), "still here").expect("write file");
        assert!(finish_checkout_removal(&occupied).is_err());
        assert!(occupied.join("locked.txt").exists());
    }
}

/// Classifies what remains at a checkout path Git no longer claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeLeftover {
    Absent,
    EmptyDirectory,
    Occupied,
}

/// Inspects a recorded checkout path after Git disowned it.
///
/// A file, a non-empty directory, or anything unreadable counts as occupied so
/// the caller never destroys content it cannot positively classify as an empty
/// shell.
fn probe_leftover(probe: &Path) -> Result<ProbeLeftover, std::io::Error> {
    match std::fs::read_dir(probe) {
        Ok(mut entries) => {
            if entries.next().is_none() {
                Ok(ProbeLeftover::EmptyDirectory)
            } else {
                Ok(ProbeLeftover::Occupied)
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ProbeLeftover::Absent),
        // A path that exists but is not a directory (NotADirectory is unstable
        // to match on across platforms) is definitely not an Ora shell.
        Err(_) if probe.exists() => Ok(ProbeLeftover::Occupied),
        Err(error) => Err(error),
    }
}

/// Confirms on disk what `git worktree remove` reported as success.
///
/// The worktree was already proven Ora-owned before this call, so a directory
/// Git deregistered but failed to fully delete may be finished with a plain
/// filesystem removal. A leftover that still has content is a retryable
/// failure rather than a silent success or an ownership question.
fn finish_checkout_removal(worktree_root: &Path) -> Result<WorktreeRemoval, GitCleanupError> {
    match probe_leftover(worktree_root).map_err(|source| {
        GitCleanupError::new(
            CleanupStage::Worktree,
            "failed to inspect checkout path after git removal",
            source,
        )
    })? {
        ProbeLeftover::Absent => Ok(WorktreeRemoval::Removed),
        ProbeLeftover::EmptyDirectory => {
            std::fs::remove_dir(worktree_root).map_err(|source| {
                GitCleanupError::new(
                    CleanupStage::Worktree,
                    "failed to remove leftover empty checkout directory",
                    source,
                )
            })?;
            Ok(WorktreeRemoval::Removed)
        }
        ProbeLeftover::Occupied => Err(GitCleanupError::new(
            CleanupStage::Worktree,
            "checkout directory still has content after git removal",
            std::io::Error::other(format!(
                "git deregistered the worktree but {} was not fully deleted",
                worktree_root.display()
            )),
        )),
    }
}

/// Normalizes a checkout root for exact comparison across canonicalization gaps.
///
/// Canonicalization succeeds for the existing side (Git-listed roots exist);
/// a probe that cannot be canonicalized falls back to its lexical form and
/// simply fails the exact match, which the caller then classifies via a plain
/// existence check.
fn normalize_root(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
