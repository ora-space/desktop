use crate::domain::worktree::WorktreeHandle;
use crate::error::{DomainError, GitExecError, GitlancerError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use crate::parse::checkpoint::{
    combine_name_status_and_numstat, parse_name_status_z, parse_numstat_z,
};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMPORARY_INDEX_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Carries the worktree and the ref name used to store one checkpoint commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotWorktreeRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub name: &'a str,
    pub message: &'a str,
}

/// Returns the commit and tree written to `refs/ora/checkpoints/<name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotWorktreeResponse {
    pub commit_oid: String,
    pub tree_oid: String,
}

/// Carries the checkpoint commit to compare against the current worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedSinceRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub commit_oid: &'a str,
}

/// One path that differs between a checkpoint commit and the current worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedPath {
    pub path: String,
    pub status: ChangeStatus,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
}

/// Classifies a checkpoint diff entry. `Renamed` keeps the pre-rename path so restore can
/// recreate it; `Other` preserves Git's raw status letter for unmapped codes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed { from: String },
    Other(String),
}

/// Returns the paths that differ between a checkpoint and the current worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedSinceResponse {
    pub entries: Vec<ChangedPath>,
}

/// Carries the checkpoint and the worktree-relative paths to restore.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestorePathsRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub commit_oid: &'a str,
    pub paths: &'a [String],
}

/// Returns which requested paths were restored from the checkpoint versus deleted from the worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestorePathsResponse {
    pub restored: Vec<String>,
    pub deleted: Vec<String>,
}

/// Carries the checkpoint whose full worktree delta should be restored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreAllRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub commit_oid: &'a str,
}

/// Returns the paths restored or deleted while rolling the worktree back to a checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreAllResponse {
    pub restored: Vec<String>,
    pub deleted: Vec<String>,
}

impl<R: GitRunner> Git<R> {
    /// Snapshots the worktree into `refs/ora/checkpoints/<name>` without touching the real index.
    pub fn snapshot_worktree(
        &self,
        request: SnapshotWorktreeRequest<'_>,
    ) -> Result<SnapshotWorktreeResponse, GitlancerError> {
        let cwd = worktree_cwd(request.worktree);
        let index = TemporaryIndex::new();
        let index_env = temp_index_env(&index);
        let has_head = self.head_exists(&cwd)?;
        if has_head {
            self.runner().run(&GitCommand::new(
                cwd.clone(),
                vec!["read-tree".to_string(), "HEAD".to_string()],
                index_env.clone(),
                GitIntent::Mutating,
            ))?;
        }
        let tree_oid = self.write_worktree_tree(&cwd, index_env)?;
        let mut commit_args = vec![
            "commit-tree".to_string(),
            tree_oid.clone(),
            "-m".to_string(),
            request.message.to_string(),
        ];
        if has_head {
            commit_args.push("-p".to_string());
            commit_args.push("HEAD".to_string());
        }
        let commit_output = self.runner().run(&GitCommand::new(
            cwd.clone(),
            commit_args,
            commit_identity_env(),
            GitIntent::Mutating,
        ))?;
        let commit_oid = trim_oid(&commit_output.stdout)?;
        self.runner().run(&GitCommand::new(
            cwd,
            vec![
                "update-ref".to_string(),
                checkpoint_ref(request.name),
                commit_oid.clone(),
            ],
            GitEnv::default(),
            GitIntent::Mutating,
        ))?;
        Ok(SnapshotWorktreeResponse {
            commit_oid,
            tree_oid,
        })
    }

    /// Lists paths that differ between a checkpoint commit and the current worktree.
    ///
    /// The comparison uses a temporary index so the caller's real index is never read or written.
    pub fn changed_since(
        &self,
        request: ChangedSinceRequest<'_>,
    ) -> Result<ChangedSinceResponse, GitlancerError> {
        let cwd = worktree_cwd(request.worktree);
        let index = TemporaryIndex::new();
        let tree_oid = self.write_worktree_tree(&cwd, temp_index_env(&index))?;
        let name_status = self
            .runner()
            .run(&GitCommand::new(
                cwd.clone(),
                vec![
                    "diff-tree".to_string(),
                    "-r".to_string(),
                    "-z".to_string(),
                    "--name-status".to_string(),
                    request.commit_oid.to_string(),
                    tree_oid.clone(),
                ],
                GitEnv::default(),
                GitIntent::ReadOnly,
            ))?
            .stdout;
        let numstat = self
            .runner()
            .run(&GitCommand::new(
                cwd,
                vec![
                    "diff-tree".to_string(),
                    "-r".to_string(),
                    "-z".to_string(),
                    "--numstat".to_string(),
                    request.commit_oid.to_string(),
                    tree_oid,
                ],
                GitEnv::default(),
                GitIntent::ReadOnly,
            ))?
            .stdout;
        let entries = combine_name_status_and_numstat(
            parse_name_status_z(&name_status)?,
            parse_numstat_z(&numstat)?,
        );
        Ok(ChangedSinceResponse { entries })
    }

    /// Restores selected worktree-relative paths from a checkpoint without touching the index.
    ///
    /// Paths that existed at the checkpoint are copied into the worktree; paths that did not are
    /// deleted. Absolute paths and `..` segments are rejected before any mutation.
    pub fn restore_paths(
        &self,
        request: RestorePathsRequest<'_>,
    ) -> Result<RestorePathsResponse, GitlancerError> {
        for path in request.paths {
            validate_worktree_relative_path(path, request.worktree)?;
        }
        let cwd = worktree_cwd(request.worktree);
        let mut restored = Vec::new();
        let mut deleted = Vec::new();
        for path in request.paths {
            if self.path_exists_in_commit(&cwd, request.commit_oid, path)? {
                self.runner().run(&GitCommand::new(
                    cwd.clone(),
                    vec![
                        "restore".to_string(),
                        format!("--source={}", request.commit_oid),
                        "--worktree".to_string(),
                        "--".to_string(),
                        path.clone(),
                    ],
                    GitEnv::default(),
                    GitIntent::Mutating,
                ))?;
                restored.push(path.clone());
            } else {
                delete_worktree_file(request.worktree, path)?;
                deleted.push(path.clone());
            }
        }
        Ok(RestorePathsResponse { restored, deleted })
    }

    /// Restores every path that differs from the checkpoint, including deleting files added since.
    pub fn restore_all(
        &self,
        request: RestoreAllRequest<'_>,
    ) -> Result<RestoreAllResponse, GitlancerError> {
        let changed = self.changed_since(ChangedSinceRequest {
            worktree: request.worktree,
            commit_oid: request.commit_oid,
        })?;
        let mut paths = Vec::new();
        for entry in changed.entries {
            match entry.status {
                ChangeStatus::Renamed { from } => {
                    paths.push(from);
                    paths.push(entry.path);
                }
                ChangeStatus::Added
                | ChangeStatus::Modified
                | ChangeStatus::Deleted
                | ChangeStatus::Other(_) => paths.push(entry.path),
            }
        }
        let RestorePathsResponse { restored, deleted } =
            self.restore_paths(RestorePathsRequest {
                worktree: request.worktree,
                commit_oid: request.commit_oid,
                paths: &paths,
            })?;
        Ok(RestoreAllResponse { restored, deleted })
    }

    /// Returns whether `HEAD` currently names a commit, treating an unborn branch as `false`.
    fn head_exists(&self, cwd: &Path) -> Result<bool, GitlancerError> {
        match self.runner().run(&GitCommand::new(
            cwd.to_path_buf(),
            vec![
                "rev-parse".to_string(),
                "--verify".to_string(),
                "--quiet".to_string(),
                "HEAD".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        )) {
            Ok(_) => Ok(true),
            Err(GitExecError::NonZeroExit { .. }) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// Stages the worktree into the isolated index and writes a tree object from it.
    fn write_worktree_tree(&self, cwd: &Path, env: GitEnv) -> Result<String, GitlancerError> {
        self.runner().run(&GitCommand::new(
            cwd.to_path_buf(),
            vec!["add".to_string(), "-A".to_string()],
            env.clone(),
            GitIntent::ReadOnly,
        ))?;
        let output = self.runner().run(&GitCommand::new(
            cwd.to_path_buf(),
            vec!["write-tree".to_string()],
            env,
            GitIntent::ReadOnly,
        ))?;
        trim_oid(&output.stdout)
    }

    /// Returns whether `<commit>:<path>` names an object, treating a missing path as `false`.
    fn path_exists_in_commit(
        &self,
        cwd: &Path,
        commit_oid: &str,
        path: &str,
    ) -> Result<bool, GitlancerError> {
        match self.runner().run(&GitCommand::new(
            cwd.to_path_buf(),
            vec![
                "cat-file".to_string(),
                "-e".to_string(),
                format!("{commit_oid}:{path}"),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        )) {
            Ok(_) => Ok(true),
            Err(GitExecError::NonZeroExit { .. }) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }
}

/// Owns a unique temporary Git index path and removes it (and any lock file) on drop.
struct TemporaryIndex {
    path: PathBuf,
}

impl TemporaryIndex {
    /// Reserves a process-unique path without creating an invalid empty index file.
    fn new() -> Self {
        let sequence = TEMPORARY_INDEX_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let process_id = std::process::id();
        Self {
            path: std::env::temp_dir()
                .join(format!("ora-checkpoint-index-{process_id}-{sequence}")),
        }
    }

    /// Returns the path passed to Git through `GIT_INDEX_FILE`.
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryIndex {
    /// Best-effort cleanup keeps checkpoint failures from accumulating temporary index files.
    fn drop(&mut self) {
        let _remove_index_result = std::fs::remove_file(&self.path);
        let lock_path = self.path.with_extension("lock");
        let _remove_lock_result = std::fs::remove_file(lock_path);
    }
}

/// Builds automation defaults that redirect Git's index to the isolated temporary file.
fn temp_index_env(index: &TemporaryIndex) -> GitEnv {
    GitEnv::default().with_variable(
        "GIT_INDEX_FILE",
        index.path().to_string_lossy().into_owned(),
    )
}

/// Fixes author and committer identity so a missing global `user.*` cannot fail `commit-tree`.
fn commit_identity_env() -> GitEnv {
    GitEnv::default()
        .with_variable("GIT_AUTHOR_NAME", "Ora")
        .with_variable("GIT_AUTHOR_EMAIL", "checkpoint@ora.local")
        .with_variable("GIT_COMMITTER_NAME", "Ora")
        .with_variable("GIT_COMMITTER_EMAIL", "checkpoint@ora.local")
}

/// Returns the worktree root Git commands must run in.
fn worktree_cwd(worktree: &WorktreeHandle) -> PathBuf {
    worktree.worktree_root().as_path().to_path_buf()
}

/// Builds the namespaced ref that stores one named checkpoint.
fn checkpoint_ref(name: &str) -> String {
    format!("refs/ora/checkpoints/{name}")
}

/// Trims a Git object id from command stdout.
fn trim_oid(stdout: &str) -> Result<String, GitlancerError> {
    let oid = stdout.trim();
    if oid.is_empty() {
        return Err(crate::ParseError::MissingLine.into());
    }
    Ok(oid.to_string())
}

/// Rejects absolute paths and any `..` segment so restore cannot escape the worktree.
fn validate_worktree_relative_path(
    path: &str,
    worktree: &WorktreeHandle,
) -> Result<(), GitlancerError> {
    let candidate = Path::new(path);
    if candidate.is_absolute()
        || candidate.has_root()
        || candidate
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(GitlancerError::Domain(DomainError::PathOutsideWorktree {
            path: candidate.to_path_buf(),
            worktree: worktree.worktree_root().as_path().to_path_buf(),
        }));
    }
    Ok(())
}

/// Deletes one worktree-relative file, ignoring an already-absent path.
fn delete_worktree_file(worktree: &WorktreeHandle, path: &str) -> Result<(), GitlancerError> {
    let full_path = worktree.worktree_root().as_path().join(path);
    match std::fs::remove_file(&full_path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
