use std::collections::BTreeMap;

use crate::domain::refs::{BranchName, CommitId};
use crate::domain::repo::Repository;
use crate::error::{DomainError, GitlancerError};
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;

const UPSTREAM_REMOTE: &str = "upstream";
const ORIGIN_REMOTE: &str = "origin";

/// Identifies a selectable worktree base without conflating its display name with its Git ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeBase {
    Local {
        branch_name: BranchName,
    },
    Remote {
        remote_name: String,
        branch_name: BranchName,
    },
}

impl WorktreeBase {
    /// Returns the logical branch name shared by local and remote-tracking refs.
    pub fn branch_name(&self) -> &BranchName {
        match self {
            Self::Local { branch_name } | Self::Remote { branch_name, .. } => branch_name,
        }
    }

    /// Returns the unambiguous ref spelling that Git should resolve for this base.
    pub fn reference_name(&self) -> String {
        match self {
            Self::Local { branch_name } => branch_name.as_str().to_string(),
            Self::Remote {
                remote_name,
                branch_name,
            } => format!("{remote_name}/{}", branch_name.as_str()),
        }
    }
}

/// Carries the repository whose current local and preferred-remote bases should be listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListWorktreeBasesRequest<'a> {
    pub repository: &'a Repository,
}

/// Returns one preferred ref per logical branch name so callers never see stale local duplicates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListWorktreeBasesResponse {
    pub bases: Vec<WorktreeBase>,
}

/// Carries the exact worktree-base ref that should be refreshed and resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveWorktreeBaseCommitRequest<'a> {
    pub repository: &'a Repository,
    pub reference_name: &'a BranchName,
}

/// Returns the immutable commit referenced by a freshly refreshed worktree base.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveWorktreeBaseCommitResponse {
    pub commit_id: CommitId,
}

impl<R: GitRunner> Git<R> {
    /// Fetches the preferred remote and merges its branches with local-only branches.
    pub fn list_worktree_bases(
        &self,
        request: ListWorktreeBasesRequest<'_>,
    ) -> Result<ListWorktreeBasesResponse, GitlancerError> {
        let remote_name = self.preferred_base_remote(request.repository)?;
        if let Some(remote_name) = remote_name.as_deref() {
            self.fetch_base_remote(request.repository, remote_name)?;
        }

        let output = self.runner().run(&build_list_worktree_bases_command(
            request.repository,
            remote_name.as_deref(),
        ))?;
        let bases = parse_worktree_bases(&output.stdout, remote_name.as_deref());

        Ok(ListWorktreeBasesResponse { bases })
    }

    /// Refreshes selectable refs again at creation time before resolving the requested base.
    pub fn resolve_worktree_base_commit(
        &self,
        request: ResolveWorktreeBaseCommitRequest<'_>,
    ) -> Result<ResolveWorktreeBaseCommitResponse, GitlancerError> {
        let bases = self.list_worktree_bases(ListWorktreeBasesRequest {
            repository: request.repository,
        })?;
        if !bases
            .bases
            .iter()
            .any(|base| base.reference_name() == request.reference_name.as_str())
        {
            return Err(GitlancerError::Domain(DomainError::BranchNotFound {
                repo: request.repository.root().as_path().to_path_buf(),
                branch: request.reference_name.as_str().to_string(),
            }));
        }

        let output = self.runner().run(&GitCommand::new(
            request.repository.root().as_path().to_path_buf(),
            vec![
                "rev-parse".to_string(),
                format!("{}^{{commit}}", request.reference_name.as_str()),
            ],
            GitEnv::default(),
            GitIntent::ReadOnly,
        ))?;
        let commit_id = crate::parse::commit::parse_commit_id(&output.stdout)?;

        Ok(ResolveWorktreeBaseCommitResponse { commit_id })
    }

    /// Selects the canonical collaboration remote without guessing among arbitrary remotes.
    fn preferred_base_remote(
        &self,
        repository: &Repository,
    ) -> Result<Option<String>, GitlancerError> {
        let output = self.runner().run(&GitCommand::new(
            repository.root().as_path().to_path_buf(),
            vec!["remote".to_string()],
            GitEnv::default(),
            GitIntent::ReadOnly,
        ))?;
        let remotes = output
            .stdout
            .lines()
            .map(str::trim)
            .filter(|remote| !remote.is_empty())
            .collect::<Vec<_>>();

        Ok(if remotes.contains(&UPSTREAM_REMOTE) {
            Some(UPSTREAM_REMOTE.to_string())
        } else if remotes.contains(&ORIGIN_REMOTE) {
            Some(ORIGIN_REMOTE.to_string())
        } else {
            None
        })
    }

    /// Updates remote-tracking refs before they are exposed as worktree bases.
    fn fetch_base_remote(
        &self,
        repository: &Repository,
        remote_name: &str,
    ) -> Result<(), GitlancerError> {
        self.runner().run(&GitCommand::new(
            repository.root().as_path().to_path_buf(),
            vec![
                "fetch".to_string(),
                "--prune".to_string(),
                remote_name.to_string(),
            ],
            GitEnv::default(),
            GitIntent::Network,
        ))?;
        Ok(())
    }
}

/// Builds one ref query whose fully qualified output preserves local-versus-remote identity.
fn build_list_worktree_bases_command(
    repository: &Repository,
    remote_name: Option<&str>,
) -> GitCommand {
    let mut args = vec![
        "for-each-ref".to_string(),
        "--format=%(refname)".to_string(),
        "refs/heads".to_string(),
    ];
    if let Some(remote_name) = remote_name {
        args.push(format!("refs/remotes/{remote_name}"));
    }

    GitCommand::new(
        repository.root().as_path().to_path_buf(),
        args,
        GitEnv::default(),
        GitIntent::ReadOnly,
    )
}

/// Parses refs into one logical branch map while preserving local Ora task branches.
///
/// Remote refs are preferred for ordinary branches because fetching is the freshness
/// boundary. Ora-managed task branches are the exception: an existing worktree may
/// contain local commits that have not been pushed, so replacing that ref with its
/// remote-tracking counterpart would make “branch from this worktree” silently lose work.
fn parse_worktree_bases(stdout: &str, remote_name: Option<&str>) -> Vec<WorktreeBase> {
    let remote_prefix = remote_name.map(|remote_name| format!("refs/remotes/{remote_name}/"));
    let mut bases = BTreeMap::<String, WorktreeBase>::new();

    for reference in stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if let Some(branch_name) = reference.strip_prefix("refs/heads/") {
            bases
                .entry(branch_name.to_string())
                .or_insert_with(|| WorktreeBase::Local {
                    branch_name: BranchName::new(branch_name),
                });
            continue;
        }

        if let (Some(remote_name), Some(remote_prefix)) = (remote_name, remote_prefix.as_deref())
            && let Some(branch_name) = reference.strip_prefix(remote_prefix)
            && branch_name != "HEAD"
        {
            if branch_name.starts_with("ora/")
                && matches!(bases.get(branch_name), Some(WorktreeBase::Local { .. }))
            {
                continue;
            }

            // A fetched remote-tracking ref is authoritative for ordinary new worktree bases.
            bases.insert(
                branch_name.to_string(),
                WorktreeBase::Remote {
                    remote_name: remote_name.to_string(),
                    branch_name: BranchName::new(branch_name),
                },
            );
        }
    }

    bases.into_values().collect()
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

    /// Captures command order while returning deterministic Git outputs.
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

        /// Returns all commands issued by the tested operation.
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

    /// Verifies upstream is fetched and its refs replace stale local duplicates.
    #[test]
    fn list_worktree_bases_prefers_fetched_upstream_refs() {
        let repository = repository_fixture();
        let git = Git::new(TestRunner::new(vec![
            output("origin\nupstream\n"),
            output(""),
            output(
                "refs/heads/local-only\nrefs/heads/main\nrefs/remotes/upstream/HEAD\nrefs/remotes/upstream/frontend\nrefs/remotes/upstream/main\n",
            ),
        ]));

        let response = git
            .list_worktree_bases(ListWorktreeBasesRequest {
                repository: &repository,
            })
            .expect("list worktree bases");

        assert_eq!(
            response.bases,
            vec![
                WorktreeBase::Remote {
                    remote_name: "upstream".to_string(),
                    branch_name: BranchName::new("frontend"),
                },
                WorktreeBase::Local {
                    branch_name: BranchName::new("local-only"),
                },
                WorktreeBase::Remote {
                    remote_name: "upstream".to_string(),
                    branch_name: BranchName::new("main"),
                },
            ]
        );
        assert_eq!(
            git.runner().recorded_commands(),
            vec![
                GitCommand::new(
                    repository.root().as_path().to_path_buf(),
                    vec!["remote".to_string()],
                    GitEnv::default(),
                    GitIntent::ReadOnly,
                ),
                GitCommand::new(
                    repository.root().as_path().to_path_buf(),
                    vec![
                        "fetch".to_string(),
                        "--prune".to_string(),
                        "upstream".to_string(),
                    ],
                    GitEnv::default(),
                    GitIntent::Network,
                ),
                build_list_worktree_bases_command(&repository, Some("upstream")),
            ]
        );
    }

    /// Verifies origin is used when the repository has no upstream remote.
    #[test]
    fn list_worktree_bases_falls_back_to_origin() {
        let repository = repository_fixture();
        let git = Git::new(TestRunner::new(vec![
            output("fork\norigin\n"),
            output(""),
            output("refs/remotes/origin/feature/runtime\n"),
        ]));

        let response = git
            .list_worktree_bases(ListWorktreeBasesRequest {
                repository: &repository,
            })
            .expect("list worktree bases");

        assert_eq!(
            response.bases,
            vec![WorktreeBase::Remote {
                remote_name: "origin".to_string(),
                branch_name: BranchName::new("feature/runtime"),
            }]
        );
        assert_eq!(git.runner().recorded_commands()[1].args[2], "origin");
    }

    /// Verifies repositories without a canonical remote keep their local branches usable.
    #[test]
    fn list_worktree_bases_supports_local_only_repositories() {
        let repository = repository_fixture();
        let git = Git::new(TestRunner::new(vec![
            output("backup\n"),
            output("refs/heads/main\n"),
        ]));

        let response = git
            .list_worktree_bases(ListWorktreeBasesRequest {
                repository: &repository,
            })
            .expect("list worktree bases");

        assert_eq!(
            response.bases,
            vec![WorktreeBase::Local {
                branch_name: BranchName::new("main"),
            }]
        );
        assert_eq!(
            git.runner().recorded_commands()[1],
            build_list_worktree_bases_command(&repository, None)
        );
    }

    /// Verifies an existing Ora worktree keeps its local tip instead of a stale remote tip.
    #[test]
    fn list_worktree_bases_preserves_local_ora_task_branches() {
        let bases = super::parse_worktree_bases(
            "refs/heads/ora/task-branch\nrefs/remotes/upstream/ora/task-branch\n",
            Some("upstream"),
        );

        assert_eq!(
            bases,
            vec![WorktreeBase::Local {
                branch_name: BranchName::new("ora/task-branch"),
            }]
        );
    }

    /// Verifies creation-time resolution fetches again and resolves the exact remote ref.
    #[test]
    fn resolve_worktree_base_commit_refreshes_the_selected_remote_ref() {
        let repository = repository_fixture();
        let git = Git::new(TestRunner::new(vec![
            output("upstream\n"),
            output(""),
            output("refs/heads/main\nrefs/remotes/upstream/main\n"),
            output("0123456789abcdef\n"),
        ]));

        let response = git
            .resolve_worktree_base_commit(ResolveWorktreeBaseCommitRequest {
                repository: &repository,
                reference_name: &BranchName::new("upstream/main"),
            })
            .expect("resolve refreshed worktree base");

        assert_eq!(response.commit_id, CommitId::new("0123456789abcdef"));
        assert_eq!(
            git.runner().recorded_commands()[3],
            GitCommand::new(
                repository.root().as_path().to_path_buf(),
                vec![
                    "rev-parse".to_string(),
                    "upstream/main^{commit}".to_string(),
                ],
                GitEnv::default(),
                GitIntent::ReadOnly,
            )
        );
    }
}
