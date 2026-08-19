use std::path::{Path, PathBuf};

use gitlancer::git::sync::{CheckoutRequest, CloneRequest, FetchRequest, PullRequest};
use gitlancer::{BranchName, Git, GitRunner, RepoRoot, Repository};

use crate::error::RegistryError;

/// Describes one marketplace source repository and where its local checkout lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrySource {
    url: String,
    branch: BranchName,
    checkout_dir: PathBuf,
}

impl RegistrySource {
    /// Creates a source bound to a git URL, a tracked branch, and a local checkout directory.
    pub fn new(
        url: impl Into<String>,
        branch: BranchName,
        checkout_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            url: url.into(),
            branch,
            checkout_dir: checkout_dir.into(),
        }
    }

    /// Returns the git URL that hosts this registry source.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the branch this source tracks.
    pub fn branch(&self) -> &BranchName {
        &self.branch
    }

    /// Returns the local directory where this source is checked out.
    pub fn checkout_dir(&self) -> &Path {
        &self.checkout_dir
    }
}

/// Syncs marketplace sources through an injected [`gitlancer::Git`] runtime.
pub struct RegistrySync;

impl RegistrySync {
    /// Ensures `source` is present and up to date: clones it when absent, otherwise fetches,
    /// checks out the tracked branch, and fast-forwards against its remote.
    ///
    /// Returns the checkout directory so callers can scan the registry contents directly.
    pub fn sync<R: GitRunner>(
        git: &Git<R>,
        source: &RegistrySource,
    ) -> Result<PathBuf, RegistryError> {
        let checkout_dir = source.checkout_dir();
        if checkout_dir.join(".git").exists() {
            let repository = Repository::new(RepoRoot::new(checkout_dir));
            git.fetch(FetchRequest {
                repository: &repository,
                remote: "origin",
            })?;
            git.checkout(CheckoutRequest {
                repository: &repository,
                branch: source.branch(),
            })?;
            git.pull(PullRequest {
                repository: &repository,
                branch: source.branch(),
            })?;
        } else {
            let parent = checkout_dir
                .parent()
                .filter(|directory| !directory.as_os_str().is_empty())
                .ok_or_else(|| RegistryError::MissingCloneParent(checkout_dir.to_path_buf()))?;
            std::fs::create_dir_all(parent)?;
            git.clone(CloneRequest {
                repository_url: source.url(),
                destination: checkout_dir.to_path_buf(),
                working_dir: parent.to_path_buf(),
                branch: Some(source.branch().clone()),
            })?;
        }
        Ok(checkout_dir.to_path_buf())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitlancer::{GitCommand, GitExecError, GitIntent, GitOutput, GitRunner};
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    /// Records every issued command so sync behavior can be asserted without executing Git.
    #[derive(Clone, Default)]
    struct RecordingRunner {
        commands: Arc<Mutex<Vec<GitCommand>>>,
    }

    impl GitRunner for RecordingRunner {
        fn run(&self, command: &GitCommand) -> Result<GitOutput, GitExecError> {
            self.commands
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(command.clone());
            Ok(GitOutput::new(Some(0), String::new(), String::new(), 0))
        }
    }

    /// Verifies an absent checkout clones the source with its tracked branch into the parent.
    #[test]
    fn clones_a_fresh_source() -> Result<(), Box<dyn std::error::Error>> {
        let runner = RecordingRunner::default();
        let git = Git::new(runner.clone());
        let temp = TempDir::new()?;
        let checkout = temp.path().join("sources").join("marketplace");
        let source = RegistrySource::new(
            "https://example.com/marketplace.git",
            BranchName::new("main"),
            &checkout,
        );

        let result = RegistrySync::sync(&git, &source)?;

        assert_eq!(result, checkout);
        let parent = checkout
            .parent()
            .ok_or_else(|| std::io::Error::other("no parent"))?;
        assert!(parent.exists());
        let commands = runner
            .commands
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].args[0], "clone");
        assert!(commands[0].args.contains(&"--branch".to_string()));
        assert!(commands[0].args.contains(&"main".to_string()));
        assert!(commands[0].args.contains(&source.url().to_string()));
        assert_eq!(commands[0].cwd, parent);
        assert_eq!(commands[0].intent, GitIntent::Network);
        Ok(())
    }

    /// Verifies an existing checkout fetches, checks out the branch, and fast-forwards its remote.
    #[test]
    fn updates_an_existing_source() -> Result<(), Box<dyn std::error::Error>> {
        let runner = RecordingRunner::default();
        let git = Git::new(runner.clone());
        let temp = TempDir::new()?;
        let checkout = temp.path().join("marketplace");
        fs::create_dir_all(checkout.join(".git"))?;
        let source = RegistrySource::new(
            "https://example.com/marketplace.git",
            BranchName::new("main"),
            &checkout,
        );

        let result = RegistrySync::sync(&git, &source)?;

        assert_eq!(result, checkout);
        let commands = runner
            .commands
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[0].args, vec!["fetch", "origin"]);
        assert_eq!(commands[0].intent, GitIntent::Network);
        assert_eq!(commands[1].args, vec!["checkout", "main"]);
        assert_eq!(commands[1].intent, GitIntent::Mutating);
        assert_eq!(
            commands[2].args,
            vec!["pull", "--ff-only", "origin", "main"]
        );
        assert_eq!(commands[2].intent, GitIntent::Network);
        Ok(())
    }
}
