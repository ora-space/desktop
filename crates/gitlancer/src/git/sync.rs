use crate::domain::refs::BranchName;
use crate::domain::repo::Repository;
use crate::error::GitlancerError;
use crate::exec::command::{GitCommand, GitIntent};
use crate::exec::env::GitEnv;
use crate::exec::runner::GitRunner;
use crate::git::Git;

/// Carries the information needed to clone a repository into a fresh directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneRequest<'a> {
    /// Git repository URL or local filesystem path to clone from.
    pub repository_url: &'a str,
    /// Directory where the clone should be created.
    pub destination: std::path::PathBuf,
    /// Existing directory the clone command runs in (the destination's parent).
    pub working_dir: std::path::PathBuf,
    /// Optional branch to check out after cloning via `--branch`.
    pub branch: Option<BranchName>,
    /// Environment applied to the clone command, usually including proxy variables.
    pub env: GitEnv,
}

/// Carries the information needed to fetch refs for an existing repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRequest<'a> {
    pub repository: &'a Repository,
    /// Remote name to fetch from, normally `origin`.
    pub remote: &'a str,
    /// Environment applied to the fetch command, usually including proxy variables.
    pub env: GitEnv,
}

/// Carries the information needed to check out a branch in an existing repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutRequest<'a> {
    pub repository: &'a Repository,
    pub branch: &'a BranchName,
}

/// Carries the information needed to fast-forward one branch against its remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest<'a> {
    pub repository: &'a Repository,
    pub branch: &'a BranchName,
    /// Environment applied to the pull command, usually including proxy variables.
    pub env: GitEnv,
}

impl<R: GitRunner> Git<R> {
    /// Clones a repository into a fresh directory so callers never build `git clone` themselves.
    pub fn clone(&self, request: CloneRequest<'_>) -> Result<(), GitlancerError> {
        self.runner().run(&build_clone_command(&request))?;
        Ok(())
    }

    /// Fetches refs from a remote so a later pull can refresh an existing checkout.
    pub fn fetch(&self, request: FetchRequest<'_>) -> Result<(), GitlancerError> {
        self.runner().run(&GitCommand::new(
            request.repository.root().as_path().to_path_buf(),
            vec!["fetch".to_string(), request.remote.to_string()],
            request.env,
            GitIntent::Network,
        ))?;
        Ok(())
    }

    /// Switches an existing checkout onto the supplied branch.
    pub fn checkout(&self, request: CheckoutRequest<'_>) -> Result<(), GitlancerError> {
        self.runner().run(&GitCommand::new(
            request.repository.root().as_path().to_path_buf(),
            vec!["checkout".to_string(), request.branch.as_str().to_string()],
            GitEnv::default(),
            GitIntent::Mutating,
        ))?;
        Ok(())
    }

    /// Fast-forwards the supplied branch against its remote so a sync converges deterministically.
    pub fn pull(&self, request: PullRequest<'_>) -> Result<(), GitlancerError> {
        self.runner().run(&GitCommand::new(
            request.repository.root().as_path().to_path_buf(),
            vec![
                "pull".to_string(),
                "--ff-only".to_string(),
                "origin".to_string(),
                request.branch.as_str().to_string(),
            ],
            request.env,
            GitIntent::Network,
        ))?;
        Ok(())
    }
}

/// Builds a stable `git clone` command so clone assembly can be tested without process execution.
pub fn build_clone_command(request: &CloneRequest<'_>) -> GitCommand {
    let mut args = vec!["clone".to_string()];
    if let Some(branch) = &request.branch {
        args.push("--branch".to_string());
        args.push(branch.as_str().to_string());
    }
    args.push(request.repository_url.to_string());
    args.push(request.destination.to_string_lossy().into_owned());

    GitCommand::new(
        request.working_dir.clone(),
        args,
        request.env.clone(),
        GitIntent::Network,
    )
}

#[cfg(test)]
mod tests {
    use super::build_clone_command;
    use crate::domain::refs::BranchName;
    use crate::exec::command::GitIntent;
    use crate::exec::env::GitEnv;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    /// Verifies a plain clone assembles `git clone <url> <destination>` in the working directory.
    #[test]
    fn assembles_plain_clone_command() {
        let command = build_clone_command(&super::CloneRequest {
            repository_url: "https://example.com/marketplace.git",
            destination: PathBuf::from("sources/marketplace"),
            working_dir: PathBuf::from("sources"),
            branch: None,
            env: GitEnv::default(),
        });

        assert_eq!(
            command.args,
            vec![
                "clone",
                "https://example.com/marketplace.git",
                "sources/marketplace"
            ]
        );
        assert_eq!(command.cwd, PathBuf::from("sources"));
        assert_eq!(command.intent, GitIntent::Network);
    }

    /// Verifies `--branch` is inserted between clone and the URL when a branch is requested.
    #[test]
    fn assembles_branch_clone_command() {
        let command = build_clone_command(&super::CloneRequest {
            repository_url: "https://example.com/marketplace.git",
            destination: PathBuf::from("sources/marketplace"),
            working_dir: PathBuf::from("sources"),
            branch: Some(BranchName::new("main")),
            env: GitEnv::default(),
        });

        assert_eq!(
            command.args,
            vec![
                "clone",
                "--branch",
                "main",
                "https://example.com/marketplace.git",
                "sources/marketplace"
            ]
        );
    }
}
