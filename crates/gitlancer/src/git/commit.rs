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

    /// Creates one commit and returns typed metadata once the commit parser is implemented.
    pub fn commit(&self, request: CommitRequest<'_>) -> Result<CommitResponse, GitlancerError> {
        let command = build_commit_command(&request);
        let _output = self.runner().run(&command)?;
        let hash_output = self.runner().run(&GitCommand::new(
            request.worktree.worktree_root().as_path().to_path_buf(),
            vec!["rev-parse".to_string(), "HEAD".to_string()],
            GitEnv::default(),
            GitIntent::ReadOnly,
        ))?;
        let summary_output = self.runner().run(&GitCommand::new(
            request.worktree.worktree_root().as_path().to_path_buf(),
            vec![
                "log".to_string(),
                "-1".to_string(),
                "--pretty=%s".to_string(),
                "HEAD".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        ))?;
        let metadata = format!("{}\n{}", hash_output.stdout, summary_output.stdout);

        parse_commit_response(&metadata).map_err(Into::into)
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
