//! Read-only facts and a mutation-only creation seam for callers that own durable recovery.
use super::worktree::{CreateWorktreeRequest, build_create_worktree_command};
use crate::{CommitId, Git, GitCommand, GitEnv, GitIntent, GitRunner, GitlancerError, Repository};
use std::path::{Path, PathBuf};

impl<R: GitRunner> Git<R> {
    /// Resolves exactly one commit without interpreting a caller reference as a Git option.
    pub fn resolve_commit(
        &self,
        repository: &Repository,
        reference: &str,
    ) -> Result<CommitId, GitlancerError> {
        let output = self.runner().run(&GitCommand::new(
            repository.root().as_path().to_path_buf(),
            vec![
                "rev-parse".into(),
                "--verify".into(),
                "--end-of-options".into(),
                format!("{reference}^{{commit}}"),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        ))?;
        Ok(crate::parse::commit::parse_commit_id(&output.stdout)?)
    }

    /// Returns the live checkout root, rejecting bare repositories and metadata-only directories.
    pub fn checkout_root(&self, checkout: &Path) -> Result<PathBuf, GitlancerError> {
        let output = self.runner().run(&GitCommand::new(
            checkout.to_path_buf(),
            vec!["rev-parse".into(), "--show-toplevel".into()],
            GitEnv::default(),
            GitIntent::ReadOnly,
        ))?;
        Ok(PathBuf::from(output.stdout.trim_end()).canonicalize()?)
    }

    /// Reads Git's actual checkout metadata directory, allowing callers to compare registration facts.
    pub fn checkout_git_directory(&self, checkout: &Path) -> Result<PathBuf, GitlancerError> {
        let output = self.runner().run(&GitCommand::new(
            checkout.to_path_buf(),
            vec!["rev-parse".into(), "--absolute-git-dir".into()],
            GitEnv::default(),
            GitIntent::ReadOnly,
        ))?;
        Ok(PathBuf::from(output.stdout.trim_end()))
    }

    /// Proves the linked metadata directory points back to this checkout and belongs to this main Git directory.
    pub fn verify_linked_checkout(
        &self,
        checkout: &Path,
        main_git_directory: &Path,
    ) -> Result<(), GitlancerError> {
        let directory = self.checkout_git_directory(checkout)?.canonicalize()?;
        let common = std::fs::read_to_string(directory.join("commondir"))?;
        let backlink = std::fs::read_to_string(directory.join("gitdir"))?;
        if directory.join(common.trim_end()).canonicalize()? != main_git_directory
            || Path::new(backlink.trim_end()).canonicalize()?
                != checkout.join(".git").canonicalize()?
            || !directory.starts_with(main_git_directory.join("worktrees"))
        {
            return Err(crate::DomainError::NotAWorktree(checkout.to_path_buf()).into());
        }
        Ok(())
    }

    /// Checks literal branch syntax; callers must not accept Git's previous-checkout expansion.
    pub fn validate_branch_name(
        &self,
        repository: &Repository,
        branch: &str,
    ) -> Result<(), GitlancerError> {
        let output = self.runner().run(&GitCommand::new(
            repository.root().as_path().to_path_buf(),
            vec!["check-ref-format".into(), "--branch".into(), branch.into()],
            GitEnv::default(),
            GitIntent::ReadOnly,
        ))?;
        if output.stdout.trim_end() != branch {
            return Err(crate::ParseError::InvalidWorktreeList.into());
        }
        Ok(())
    }

    /// Issues only `worktree add`; durable callers retain partial effects for ownership-aware recovery.
    ///
    /// Unlike `create_worktree`, this method never compensates a failed observation with deletion.
    /// The caller must persist its intent first and verify the resulting Git facts independently.
    pub fn create_worktree_for_recovery(
        &self,
        request: CreateWorktreeRequest<'_>,
    ) -> Result<(), GitlancerError> {
        self.runner()
            .run(&build_create_worktree_command(&request))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::{BranchName, GitExecError, GitOutput, RepoRoot, WorktreeRoot};
    use pretty_assertions::assert_eq;
    use std::cell::RefCell;

    struct Runner {
        commands: RefCell<Vec<GitCommand>>,
        output: Result<GitOutput, GitExecError>,
    }
    impl GitRunner for Runner {
        /// Records the full invocation and returns the single configured observation.
        fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
            self.commands.borrow_mut().push(command.clone());
            match &self.output {
                Ok(output) => Ok(output.clone()),
                Err(_) => Err(GitExecError::NonZeroExit {
                    code: Some(1),
                    args: command.args.clone(),
                    stdout: String::new(),
                    stderr: "interrupted".into(),
                }),
            }
        }
    }

    /// A failed add must leave all compensation decisions to the durable owner.
    #[test]
    fn recovery_creation_never_runs_implicit_cleanup() {
        let git = Git::new(Runner {
            commands: RefCell::default(),
            output: Err(GitExecError::NonZeroExit {
                code: Some(1),
                args: vec![],
                stdout: String::new(),
                stderr: "fail".into(),
            }),
        });
        let repo = Repository::new(RepoRoot::new(std::env::temp_dir()));
        assert!(
            git.create_worktree_for_recovery(CreateWorktreeRequest {
                repository: &repo,
                worktree_root: WorktreeRoot::new(std::env::temp_dir().join("task")),
                branch_name: BranchName::new("task"),
                base_commit_id: CommitId::new("abc")
            })
            .is_err()
        );
        assert_eq!(git.runner().commands.borrow().len(), 1);
        assert_eq!(
            git.runner().commands.borrow()[0].intent,
            GitIntent::Mutating
        );
    }

    /// Option-like references stay behind end-of-options and resolve to immutable full commit IDs.
    #[test]
    fn commit_resolution_is_option_safe() {
        let commit = "a".repeat(/*n*/ 40);
        let git = Git::new(Runner {
            commands: RefCell::default(),
            output: Ok(GitOutput::new(
                Some(0),
                format!("{commit}\n"),
                String::new(),
                0,
            )),
        });
        let repo = Repository::new(RepoRoot::new(std::env::temp_dir()));
        assert_eq!(
            git.resolve_commit(&repo, "--help").unwrap(),
            CommitId::new(commit)
        );
        assert_eq!(
            git.runner().commands.borrow()[0].args,
            vec![
                "rev-parse",
                "--verify",
                "--end-of-options",
                "--help^{commit}"
            ]
        );
    }
}
