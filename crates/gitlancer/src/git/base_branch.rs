use crate::domain::refs::{BranchName, CommitId};
use crate::domain::repo::Repository;
use crate::error::{DomainError, GitExecError, GitlancerError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;

/// Identifies a selectable local branch without conflating its display name with its Git ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeBase {
    Local { branch_name: BranchName },
}

impl WorktreeBase {
    /// Returns the branch name shown to callers and used when resolving the local ref.
    pub fn branch_name(&self) -> &BranchName {
        match self {
            Self::Local { branch_name } => branch_name,
        }
    }

    /// Returns the local ref spelling that Git should resolve for this base.
    pub fn reference_name(&self) -> String {
        self.branch_name().as_str().to_string()
    }
}

/// Carries the repository whose local branch bases should be listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListWorktreeBasesRequest<'a> {
    pub repository: &'a Repository,
}

/// Returns the local branch refs available as worktree bases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListWorktreeBasesResponse {
    pub bases: Vec<WorktreeBase>,
}

/// Carries the selected local branch ref that should be resolved to an immutable commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveWorktreeBaseCommitRequest<'a> {
    pub repository: &'a Repository,
    pub reference_name: &'a BranchName,
}

/// Returns the immutable commit referenced by a local worktree base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveWorktreeBaseCommitResponse {
    pub commit_id: CommitId,
}

impl<R: GitRunner> Git<R> {
    /// Lists only local branch refs so opening the selector never changes repository state or needs a network.
    pub fn list_worktree_bases(
        &self,
        request: ListWorktreeBasesRequest<'_>,
    ) -> Result<ListWorktreeBasesResponse, GitlancerError> {
        let output = self
            .runner()
            .run(&build_list_worktree_bases_command(request.repository))?;
        let bases = parse_worktree_bases(&output.stdout);

        Ok(ListWorktreeBasesResponse { bases })
    }

    /// Resolves the selected local branch directly without refreshing or merging remote refs.
    pub fn resolve_worktree_base_commit(
        &self,
        request: ResolveWorktreeBaseCommitRequest<'_>,
    ) -> Result<ResolveWorktreeBaseCommitResponse, GitlancerError> {
        let output = self
            .runner()
            .run(&GitCommand::new(
                request.repository.root().as_path().to_path_buf(),
                vec![
                    "rev-parse".to_string(),
                    format!("{}^{{commit}}", request.reference_name.as_str()),
                ],
                GitEnv::default(),
                GitIntent::ReadOnly,
            ))
            .map_err(|error| match error {
                GitExecError::NonZeroExit { .. } => {
                    GitlancerError::Domain(DomainError::BranchNotFound {
                        repo: request.repository.root().as_path().to_path_buf(),
                        branch: request.reference_name.as_str().to_string(),
                    })
                }
                other => GitlancerError::Exec(other),
            })?;
        let commit_id = crate::parse::commit::parse_commit_id(&output.stdout)?;

        Ok(ResolveWorktreeBaseCommitResponse { commit_id })
    }
}

/// Builds the read-only ref query used to enumerate local worktree bases.
fn build_list_worktree_bases_command(repository: &Repository) -> GitCommand {
    GitCommand::new(
        repository.root().as_path().to_path_buf(),
        vec![
            "for-each-ref".to_string(),
            "--format=%(refname:short)".to_string(),
            "refs/heads".to_string(),
        ],
        GitEnv::default(),
        GitIntent::ReadOnly,
    )
}

/// Converts Git's short local branch output into typed worktree bases.
fn parse_worktree_bases(stdout: &str) -> Vec<WorktreeBase> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|branch_name| WorktreeBase::Local {
            branch_name: BranchName::new(branch_name),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use pretty_assertions::assert_eq;

    use super::{
        ListWorktreeBasesRequest, ResolveWorktreeBaseCommitRequest, WorktreeBase,
        build_list_worktree_bases_command,
    };
    use crate::domain::paths::RepoRoot;
    use crate::domain::refs::{BranchName, CommitId};
    use crate::domain::repo::Repository;
    use crate::exec::command::{GitCommand, GitIntent};
    use crate::exec::output::GitOutput;
    use crate::exec::runner::GitRunner;
    use crate::git::Git;
    use crate::{GitEnv, GitExecError};

    /// Captures commands while returning deterministic Git outputs.
    #[derive(Debug, Default)]
    struct TestRunner {
        outputs: RefCell<Vec<GitOutput>>,
        commands: RefCell<Vec<GitCommand>>,
    }

    impl TestRunner {
        /// Creates a runner whose outputs are consumed in call order.
        fn new(outputs: Vec<GitOutput>) -> Self {
            Self {
                outputs: RefCell::new(outputs.into_iter().rev().collect()),
                commands: RefCell::new(Vec::new()),
            }
        }

        /// Returns every command issued by the tested operation.
        fn recorded_commands(&self) -> Vec<GitCommand> {
            self.commands.borrow().clone()
        }
    }

    impl GitRunner for TestRunner {
        /// Records each command before returning its queued output.
        fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
            self.commands.borrow_mut().push(command.clone());
            Ok(self
                .outputs
                .borrow_mut()
                .pop()
                .unwrap_or_else(|| GitOutput::new(Some(0), String::new(), String::new(), 0)))
        }
    }

    /// Creates a stable repository handle for command-level tests.
    fn repository_fixture() -> Repository {
        Repository::new(RepoRoot::new("/repo"))
    }

    /// Creates a successful output without irrelevant stderr or timing data.
    fn output(stdout: &str) -> GitOutput {
        GitOutput::new(Some(0), stdout.to_string(), String::new(), 0)
    }

    /// Verifies branch listing reads only local heads and never probes or updates a remote.
    #[test]
    fn list_worktree_bases_lists_only_local_refs() {
        let repository = repository_fixture();
        let git = Git::new(TestRunner::new(vec![output("main\nfeature/runtime\n")]));

        let response = git
            .list_worktree_bases(ListWorktreeBasesRequest {
                repository: &repository,
            })
            .expect("list local worktree bases");

        assert_eq!(
            response.bases,
            vec![
                WorktreeBase::Local {
                    branch_name: BranchName::new("main"),
                },
                WorktreeBase::Local {
                    branch_name: BranchName::new("feature/runtime"),
                },
            ]
        );
        assert_eq!(
            git.runner().recorded_commands(),
            vec![build_list_worktree_bases_command(&repository)]
        );
    }

    /// Verifies creation-time resolution directly reads the selected local ref with one Git command.
    #[test]
    fn resolve_worktree_base_commit_reads_the_selected_local_ref() {
        let repository = repository_fixture();
        let git = Git::new(TestRunner::new(vec![output("0123456789abcdef\n")]));

        let response = git
            .resolve_worktree_base_commit(ResolveWorktreeBaseCommitRequest {
                repository: &repository,
                reference_name: &BranchName::new("main"),
            })
            .expect("resolve local worktree base");

        assert_eq!(response.commit_id, CommitId::new("0123456789abcdef"));
        assert_eq!(
            git.runner().recorded_commands(),
            vec![GitCommand::new(
                repository.root().as_path().to_path_buf(),
                vec!["rev-parse".to_string(), "main^{commit}".to_string()],
                GitEnv::default(),
                GitIntent::ReadOnly,
            )]
        );
    }
}
