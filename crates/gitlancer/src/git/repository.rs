use crate::domain::paths::RepoRoot;
use crate::domain::repo::Repository;
use crate::error::{DomainError, GitExecError, GitlancerError, ParseError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;
use crate::git::worktree::{ListWorktreesResponse, ListWorktreesResult};
use crate::parse;

/// Carries the information needed to list worktrees for one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListWorktreesRequest<'a> {
    pub repository: &'a Repository,
}

impl<R: GitRunner> Git<R> {
    /// Creates a repository handle from a validated root so use cases can accept repository identity explicitly.
    pub fn open_repository(&self, root: RepoRoot) -> Repository {
        Repository::new(root)
    }

    /// Lists worktrees for one repository using Git's porcelain worktree listing format.
    pub fn list_worktrees(
        &self,
        request: ListWorktreesRequest<'_>,
    ) -> Result<ListWorktreesResponse, GitlancerError> {
        let command = GitCommand::new(
            request.repository.root().as_path().to_path_buf(),
            vec![
                "worktree".to_string(),
                "list".to_string(),
                "--porcelain".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        );
        let output = self.runner().run(&command)?;
        let worktrees =
            parse::worktree::parse_worktree_list(request.repository.root(), &output.stdout)?;

        Ok(ListWorktreesResult { worktrees }.into())
    }

    /// Discovers the owning repository root from any worktree by reading the main checkout from Git's worktree list.
    pub fn discover_repository(&self, root: RepoRoot) -> Result<Repository, GitlancerError> {
        let command = GitCommand::new(
            root.as_path().to_path_buf(),
            vec![
                "worktree".to_string(),
                "list".to_string(),
                "--porcelain".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        );
        let output = self.runner().run(&command).map_err(|error| match error {
            GitExecError::NonZeroExit { .. } => {
                GitlancerError::Domain(DomainError::NotARepository(root.as_path().to_path_buf()))
            }
            other => GitlancerError::Exec(other),
        })?;
        let top_level = output
            .stdout
            .lines()
            .find_map(|line| line.trim().strip_prefix("worktree "))
            .ok_or(ParseError::InvalidWorktreeList)?;

        Ok(Repository::new(RepoRoot::new(top_level)))
    }
}
