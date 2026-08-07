use crate::domain::paths::RepoRelativePath;
use crate::domain::refs::CommitId;
use crate::domain::worktree::WorktreeHandle;
use crate::error::GitlancerError;
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use crate::parse::commit::parse_commit_response;

/// Carries the information needed to stage one or more repo-relative paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub paths: Vec<RepoRelativePath>,
}

/// Returns the paths that were requested for staging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddResponse {
    pub staged_paths: Vec<RepoRelativePath>,
}

/// Carries the worktree whose complete change set should be staged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageAllRequest<'a> {
    pub worktree: &'a WorktreeHandle,
}

/// Carries the repo-relative paths whose index entries should be removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnstageRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub paths: Vec<RepoRelativePath>,
}

/// Carries the worktree whose complete index should be restored to HEAD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnstageAllRequest<'a> {
    pub worktree: &'a WorktreeHandle,
}

/// Carries the information needed to create a commit in one worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest<'a> {
    pub worktree: &'a WorktreeHandle,
    pub message: &'a str,
    pub allow_empty: bool,
}

/// Returns the typed metadata upper layers typically need after a successful commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitResponse {
    pub commit_id: CommitId,
    pub summary: String,
}

impl<R: GitRunner> Git<R> {
    /// Stages repo-relative paths so callers never need to build `git add` commands themselves.
    pub fn add(&self, request: AddRequest<'_>) -> Result<AddResponse, GitlancerError> {
        let command = build_add_command(&request);
        let _output = self.runner().run(&command)?;

        Ok(AddResponse {
            staged_paths: request.paths,
        })
    }

    /// Stages tracked changes, deletions, and untracked files for an explicit task commit.
    pub fn stage_all(&self, request: StageAllRequest<'_>) -> Result<(), GitlancerError> {
        self.runner().run(&GitCommand::new(
            request.worktree.worktree_root().as_path().to_path_buf(),
            vec![
                "add".to_string(),
                "--all".to_string(),
                "--".to_string(),
                ".".to_string(),
            ],
            GitEnv::default(),
            GitIntent::Mutating,
        ))?;
        Ok(())
    }

    /// Removes selected paths from the index while preserving their worktree content.
    pub fn unstage(&self, request: UnstageRequest<'_>) -> Result<(), GitlancerError> {
        self.runner().run(&build_unstage_command(&request))?;
        Ok(())
    }

    /// Removes every staged path from the index while preserving worktree content.
    pub fn unstage_all(&self, request: UnstageAllRequest<'_>) -> Result<(), GitlancerError> {
        self.runner().run(&build_unstage_all_command(&request))?;
        Ok(())
    }

    /// Creates one commit and returns typed metadata once the commit parser is implemented.
    pub fn commit(&self, request: CommitRequest<'_>) -> Result<CommitResponse, GitlancerError> {
        let command = build_commit_command(&request);
        let _output = self.runner().run(&command)?;
        let hash_output = self
            .runner()
            .run(&GitCommand::new(
                request.worktree.worktree_root().as_path().to_path_buf(),
                vec!["rev-parse".to_string(), "HEAD".to_string()],
                GitEnv::default(),
                GitIntent::ReadOnly,
            ))
            .map_err(|source| GitlancerError::CommitMetadataUnavailable {
                source: Box::new(GitlancerError::Exec(source)),
            })?;
        let summary_output = self
            .runner()
            .run(&GitCommand::new(
                request.worktree.worktree_root().as_path().to_path_buf(),
                vec![
                    "log".to_string(),
                    "-1".to_string(),
                    "--pretty=%s".to_string(),
                    "HEAD".to_string(),
                ],
                GitEnv::default(),
                GitIntent::ReadOnly,
            ))
            .map_err(|source| GitlancerError::CommitMetadataUnavailable {
                source: Box::new(GitlancerError::Exec(source)),
            })?;
        // Keep an explicit empty second line so an intentionally empty summary is still a valid response.
        let metadata = format!(
            "{}\n{}\n",
            hash_output.stdout.trim_end(),
            summary_output.stdout.trim_end()
        );

        parse_commit_response(&metadata)
            .map_err(GitlancerError::from)
            .map_err(|source| GitlancerError::CommitMetadataUnavailable {
                source: Box::new(source),
            })
    }
}

/// Builds a stable `git add` command so staging behavior can be tested independently from process execution.
pub fn build_add_command(request: &AddRequest<'_>) -> GitCommand {
    let mut args = vec!["add".to_string(), "--".to_string()];
    args.extend(
        request
            .paths
            .iter()
            .map(|path| path.as_path().to_string_lossy().into_owned()),
    );

    GitCommand::new(
        request.worktree.worktree_root().as_path().to_path_buf(),
        args,
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

/// Builds a stable `git commit` command so commit policy and options stay centralized.
pub fn build_commit_command(request: &CommitRequest<'_>) -> GitCommand {
    let mut args = vec![
        "commit".to_string(),
        "--no-gpg-sign".to_string(),
        "-m".to_string(),
        request.message.to_string(),
    ];

    if request.allow_empty {
        args.push("--allow-empty".to_string());
    }

    GitCommand::new(
        request.worktree.worktree_root().as_path().to_path_buf(),
        args,
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

/// Builds the selected-path restore command used to unstage without discarding edits.
pub fn build_unstage_command(request: &UnstageRequest<'_>) -> GitCommand {
    let mut args = vec![
        "restore".to_string(),
        "--staged".to_string(),
        "--".to_string(),
    ];
    args.extend(
        request
            .paths
            .iter()
            .map(|path| path.as_path().to_string_lossy().into_owned()),
    );

    GitCommand::new(
        request.worktree.worktree_root().as_path().to_path_buf(),
        args,
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

/// Builds the all-path restore command used to unstage without discarding edits.
pub fn build_unstage_all_command(request: &UnstageAllRequest<'_>) -> GitCommand {
    GitCommand::new(
        request.worktree.worktree_root().as_path().to_path_buf(),
        vec![
            "restore".to_string(),
            "--staged".to_string(),
            "--".to_string(),
            ".".to_string(),
        ],
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::{
        AddRequest, UnstageAllRequest, UnstageRequest, build_add_command,
        build_unstage_all_command, build_unstage_command,
    };
    use crate::domain::paths::{GitDir, RepoRelativePath, RepoRoot, WorktreeRoot};
    use crate::domain::refs::BranchName;
    use crate::domain::worktree::{WorktreeHandle, WorktreeKind};

    /// Creates a stable main-worktree fixture for command assembly tests.
    fn worktree_fixture() -> WorktreeHandle {
        let root = RepoRoot::new("D:/gitlancer-commit-tests");
        WorktreeHandle::new(
            root.clone(),
            WorktreeRoot::new(root.as_path()),
            GitDir::new(root.as_path().join(".git")),
            WorktreeKind::Main,
            Some(BranchName::new("main")),
        )
    }

    /// Verifies selected staging and unstaging commands keep path arguments after `--`.
    #[test]
    fn builds_selected_change_commands() {
        let worktree = worktree_fixture();
        let path = RepoRelativePath::new("src/main.rs");

        assert_eq!(
            build_add_command(&AddRequest {
                worktree: &worktree,
                paths: vec![path.clone()],
            })
            .args,
            vec!["add", "--", "src/main.rs"]
        );
        assert_eq!(
            build_unstage_command(&UnstageRequest {
                worktree: &worktree,
                paths: vec![path],
            })
            .args,
            vec!["restore", "--staged", "--", "src/main.rs"]
        );
    }

    /// Verifies all-path unstaging uses a scoped restore command instead of discarding edits.
    #[test]
    fn builds_all_unstage_command() {
        let worktree = worktree_fixture();

        assert_eq!(
            build_unstage_all_command(&UnstageAllRequest {
                worktree: &worktree,
            })
            .args,
            vec!["restore", "--staged", "--", "."]
        );
    }
}
