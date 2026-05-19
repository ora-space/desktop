use crate::domain::refs::BranchName;
use crate::domain::repo::Repository;
use crate::error::{DomainError, GitlancerError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;

/// Carries the information needed to read branch names from a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListBranchesRequest<'a> {
    pub repository: &'a Repository,
}

/// Returns branch names in a typed form so upper layers can avoid raw-string plumbing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListBranchesResponse {
    pub branches: Vec<BranchName>,
}

/// Carries the information needed to create one local branch in a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateBranchRequest<'a> {
    pub repository: &'a Repository,
    pub branch_name: BranchName,
}

/// Returns the branch created through the runtime API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateBranchResponse {
    pub branch: BranchName,
}

/// Describes how branch deletion should behave when Git would otherwise protect the branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchDeletionMode {
    Checked,
    Force,
}

/// Carries the information needed to delete one local branch from a repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteBranchRequest<'a> {
    pub repository: &'a Repository,
    pub branch_name: BranchName,
    pub mode: BranchDeletionMode,
}

/// Returns the branch deleted through the runtime API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteBranchResponse {
    pub branch: BranchName,
}

impl<R: GitRunner> Git<R> {
    /// Lists local branches from Git's ref database so callers can avoid parsing human-oriented branch output.
    pub fn list_branches(
        &self,
        request: ListBranchesRequest<'_>,
    ) -> Result<ListBranchesResponse, GitlancerError> {
        let command = GitCommand::new(
            request.repository.root().as_path().to_path_buf(),
            vec![
                "for-each-ref".to_string(),
                "--format=%(refname:short)".to_string(),
                "refs/heads".to_string(),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        );
        let output = self.runner().run(&command)?;
        let branches = output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| BranchName::new(line.to_string()))
            .collect();

        Ok(ListBranchesResponse { branches })
    }

    /// Creates a new local branch after validating the target branch name is not already present.
    pub fn create_branch(
        &self,
        request: CreateBranchRequest<'_>,
    ) -> Result<CreateBranchResponse, GitlancerError> {
        let existing_branches = self.list_branches(ListBranchesRequest {
            repository: request.repository,
        })?;
        if existing_branches
            .branches
            .iter()
            .any(|branch| branch == &request.branch_name)
        {
            return Err(GitlancerError::Domain(DomainError::BranchAlreadyExists {
                repo: request.repository.root().as_path().to_path_buf(),
                branch: request.branch_name.as_str().to_string(),
            }));
        }

        let command = build_create_branch_command(&request);
        let _output = self.runner().run(&command)?;

        Ok(CreateBranchResponse {
            branch: request.branch_name,
        })
    }

    /// Deletes one local branch after validating the named branch exists in the supplied repository.
    pub fn delete_branch(
        &self,
        request: DeleteBranchRequest<'_>,
    ) -> Result<DeleteBranchResponse, GitlancerError> {
        let existing_branches = self.list_branches(ListBranchesRequest {
            repository: request.repository,
        })?;
        if !existing_branches
            .branches
            .iter()
            .any(|branch| branch == &request.branch_name)
        {
            return Err(GitlancerError::Domain(DomainError::BranchNotFound {
                repo: request.repository.root().as_path().to_path_buf(),
                branch: request.branch_name.as_str().to_string(),
            }));
        }

        let command = build_delete_branch_command(&request);
        let _output = self.runner().run(&command)?;

        Ok(DeleteBranchResponse {
            branch: request.branch_name,
        })
    }
}

/// Builds a stable `git branch` command for local branch creation so tests can verify lifecycle semantics directly.
pub fn build_create_branch_command(request: &CreateBranchRequest<'_>) -> GitCommand {
    GitCommand::new(
        request.repository.root().as_path().to_path_buf(),
        vec![
            "branch".to_string(),
            request.branch_name.as_str().to_string(),
        ],
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

/// Builds a stable `git branch -d/-D` command so deletion policy remains explicit and testable.
pub fn build_delete_branch_command(request: &DeleteBranchRequest<'_>) -> GitCommand {
    let delete_flag = match request.mode {
        BranchDeletionMode::Checked => "-d",
        BranchDeletionMode::Force => "-D",
    };

    GitCommand::new(
        request.repository.root().as_path().to_path_buf(),
        vec![
            "branch".to_string(),
            delete_flag.to_string(),
            request.branch_name.as_str().to_string(),
        ],
        GitEnv::default(),
        GitIntent::Mutating,
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use pretty_assertions::assert_eq;

    use super::{
        BranchDeletionMode, CreateBranchRequest, DeleteBranchRequest, build_create_branch_command,
        build_delete_branch_command,
    };
    use crate::domain::paths::RepoRoot;
    use crate::domain::refs::BranchName;
    use crate::domain::repo::Repository;
    use crate::error::{DomainError, GitExecError, GitlancerError};
    use crate::exec::command::{GitCommand, GitIntent};
    use crate::exec::output::GitOutput;
    use crate::exec::runner::GitRunner;
    use crate::git::{Git, branch::ListBranchesRequest};

    /// Captures Git commands and returns queued outputs so lifecycle tests can verify assembly and intent.
    #[derive(Debug, Default)]
    struct TestRunner {
        outputs: RefCell<Vec<GitOutput>>,
        commands: RefCell<Vec<GitCommand>>,
    }

    impl TestRunner {
        /// Creates a test runner whose queued outputs are consumed in call order.
        fn new(outputs: Vec<GitOutput>) -> Self {
            Self {
                outputs: RefCell::new(outputs.into_iter().rev().collect()),
                commands: RefCell::new(Vec::new()),
            }
        }

        /// Returns every command the runtime attempted to execute.
        fn recorded_commands(&self) -> Vec<GitCommand> {
            self.commands.borrow().clone()
        }
    }

    impl GitRunner for TestRunner {
        /// Records each command while returning the next queued output to keep tests deterministic.
        fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
            self.commands.borrow_mut().push(command.clone());
            Ok(self
                .outputs
                .borrow_mut()
                .pop()
                .unwrap_or_else(|| GitOutput::new(Some(0), String::new(), String::new(), 0)))
        }
    }

    /// Creates a stable repository fixture for branch lifecycle tests.
    fn repository_fixture() -> Repository {
        Repository::new(RepoRoot::new("/tmp/gitlancer-branch-tests"))
    }

    /// Verifies branch listing still assembles the same read-only command after lifecycle APIs are added.
    #[test]
    fn list_branches_builds_a_read_only_ref_query() {
        let repository = repository_fixture();
        let git = Git::new(TestRunner::new(vec![GitOutput::new(
            Some(0),
            "main\nfeature/runtime\n".to_string(),
            String::new(),
            0,
        )]));

        let response = git
            .list_branches(ListBranchesRequest {
                repository: &repository,
            })
            .expect("list branches");

        assert_eq!(
            response.branches,
            vec![BranchName::new("main"), BranchName::new("feature/runtime")]
        );
        assert_eq!(
            git.runner().recorded_commands(),
            vec![GitCommand::new(
                repository.root().as_path().to_path_buf(),
                vec![
                    "for-each-ref".to_string(),
                    "--format=%(refname:short)".to_string(),
                    "refs/heads".to_string(),
                ],
                crate::GitEnv::default(),
                GitIntent::ReadOnly,
            )]
        );
    }

    /// Verifies local branch creation validates then issues one mutating `git branch` command.
    #[test]
    fn create_branch_validates_existing_refs_before_creating() {
        let repository = repository_fixture();
        let runner = TestRunner::new(vec![
            GitOutput::new(Some(0), "main\n".to_string(), String::new(), 0),
            GitOutput::new(Some(0), String::new(), String::new(), 0),
        ]);
        let git = Git::new(runner);

        let response = git
            .create_branch(CreateBranchRequest {
                repository: &repository,
                branch_name: BranchName::new("feature/runtime"),
            })
            .expect("create branch");

        assert_eq!(response.branch, BranchName::new("feature/runtime"));
        assert_eq!(
            git.runner().recorded_commands(),
            vec![
                GitCommand::new(
                    repository.root().as_path().to_path_buf(),
                    vec![
                        "for-each-ref".to_string(),
                        "--format=%(refname:short)".to_string(),
                        "refs/heads".to_string(),
                    ],
                    crate::GitEnv::default(),
                    GitIntent::ReadOnly,
                ),
                GitCommand::new(
                    repository.root().as_path().to_path_buf(),
                    vec!["branch".to_string(), "feature/runtime".to_string()],
                    crate::GitEnv::default(),
                    GitIntent::Mutating,
                ),
            ]
        );
    }

    /// Verifies duplicate branch creation fails before the runtime attempts a mutating Git command.
    #[test]
    fn create_branch_rejects_duplicates_before_execution() {
        let repository = repository_fixture();
        let git = Git::new(TestRunner::new(vec![GitOutput::new(
            Some(0),
            "main\nfeature/runtime\n".to_string(),
            String::new(),
            0,
        )]));

        let error = git
            .create_branch(CreateBranchRequest {
                repository: &repository,
                branch_name: BranchName::new("feature/runtime"),
            })
            .expect_err("duplicate branch should be rejected");

        assert!(
            matches!(
                error,
                GitlancerError::Domain(DomainError::BranchAlreadyExists { repo, branch })
                    if repo == repository.root().as_path() && branch == "feature/runtime"
            ),
            "duplicate branches should fail with BranchAlreadyExists"
        );
        assert_eq!(git.runner().recorded_commands().len(), 1);
    }

    /// Verifies local branch deletion validates existence and uses the explicit deletion mode in command assembly.
    #[test]
    fn delete_branch_validates_existence_and_uses_the_selected_mode() {
        let repository = repository_fixture();
        let runner = TestRunner::new(vec![
            GitOutput::new(
                Some(0),
                "main\nfeature/runtime\n".to_string(),
                String::new(),
                0,
            ),
            GitOutput::new(Some(0), String::new(), String::new(), 0),
        ]);
        let git = Git::new(runner);

        let response = git
            .delete_branch(DeleteBranchRequest {
                repository: &repository,
                branch_name: BranchName::new("feature/runtime"),
                mode: BranchDeletionMode::Force,
            })
            .expect("delete branch");

        assert_eq!(response.branch, BranchName::new("feature/runtime"));
        assert_eq!(
            git.runner().recorded_commands()[1],
            GitCommand::new(
                repository.root().as_path().to_path_buf(),
                vec![
                    "branch".to_string(),
                    "-D".to_string(),
                    "feature/runtime".to_string(),
                ],
                crate::GitEnv::default(),
                GitIntent::Mutating,
            )
        );
    }

    /// Verifies missing local branches are rejected before a delete command can be issued.
    #[test]
    fn delete_branch_rejects_unknown_branches_before_execution() {
        let repository = repository_fixture();
        let git = Git::new(TestRunner::new(vec![GitOutput::new(
            Some(0),
            "main\n".to_string(),
            String::new(),
            0,
        )]));

        let error = git
            .delete_branch(DeleteBranchRequest {
                repository: &repository,
                branch_name: BranchName::new("feature/runtime"),
                mode: BranchDeletionMode::Checked,
            })
            .expect_err("unknown branch should be rejected");

        assert!(
            matches!(
                error,
                GitlancerError::Domain(DomainError::BranchNotFound { repo, branch })
                    if repo == repository.root().as_path() && branch == "feature/runtime"
            ),
            "unknown branches should fail with BranchNotFound"
        );
        assert_eq!(git.runner().recorded_commands().len(), 1);
    }

    /// Verifies the dedicated command builders stay stable for tests and downstream callers.
    #[test]
    fn branch_command_builders_encode_modes_explicitly() {
        let repository = repository_fixture();

        let create_command = build_create_branch_command(&CreateBranchRequest {
            repository: &repository,
            branch_name: BranchName::new("feature/runtime"),
        });
        let delete_command = build_delete_branch_command(&DeleteBranchRequest {
            repository: &repository,
            branch_name: BranchName::new("feature/runtime"),
            mode: BranchDeletionMode::Checked,
        });

        assert_eq!(
            create_command,
            GitCommand::new(
                repository.root().as_path().to_path_buf(),
                vec!["branch".to_string(), "feature/runtime".to_string()],
                crate::GitEnv::default(),
                GitIntent::Mutating,
            )
        );
        assert_eq!(
            delete_command,
            GitCommand::new(
                repository.root().as_path().to_path_buf(),
                vec![
                    "branch".to_string(),
                    "-d".to_string(),
                    "feature/runtime".to_string(),
                ],
                crate::GitEnv::default(),
                GitIntent::Mutating,
            )
        );
    }
}
