use std::path::{Path, PathBuf};

use gitlancer::git::sync::{CheckoutRequest, CloneRequest, FetchRequest, PullRequest};
use gitlancer::{BranchName, Git, GitEnv, GitRunner, RepoRoot, Repository};
use ora_domain::PluginNamespace;
use ora_plugin_manifest::RepositoryUrl;
use ora_utils::GitBranchName;
use ora_utils::url::canonical_repository_url;

use crate::error::RegistryError;

/// Directory inside a source checkout that holds the published plugin entries.
const REGISTRY_DIRECTORY: &str = "registry";

/// Describes one marketplace source repository, the namespace it publishes under, and where its
/// local checkout lives.
///
/// The namespace is supplied by the caller rather than derived here: it is bound once when the
/// source is first configured and then persisted, so a source that is removed and re-added, or
/// whose URL is respelled, keeps publishing under the identity its already-installed plugins were
/// installed with. Deriving it on every construction would let that identity drift.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrySource {
    url: String,
    canonical_url: String,
    namespace: PluginNamespace,
    branch: BranchName,
    checkout_dir: PathBuf,
    git_env: GitEnv,
}

impl RegistrySource {
    /// Creates a source bound to a git URL, a namespace, a tracked branch, and a local checkout
    /// directory.
    pub fn new(
        url: impl Into<String>,
        namespace: PluginNamespace,
        branch: BranchName,
        checkout_dir: impl Into<PathBuf>,
    ) -> Self {
        let url = url.into();
        let canonical_url = canonical_repository_url(&url);
        Self {
            url,
            canonical_url,
            namespace,
            branch,
            checkout_dir: checkout_dir.into(),
            git_env: GitEnv::default(),
        }
    }

    /// Creates a source from a git URL and tracked branch, deriving its local checkout directory
    /// from the canonical URL beneath `sources_root`.
    ///
    /// Deriving the directory from the URL keeps additional marketplace sources distinct without
    /// a manual URL-to-directory mapping, and reproduces the layout that predates multiple
    /// sources: the scheme is stripped and the remainder is joined as path segments, so
    /// `https://github.com/ora-space/marketplace` checks out at
    /// `<sources_root>/github.com/ora-space/marketplace`. Canonicalizing first means two
    /// equivalent spellings of one repository share a single checkout instead of cloning twice.
    pub fn from_git(
        url: impl Into<String>,
        namespace: PluginNamespace,
        branch: BranchName,
        sources_root: impl AsRef<Path>,
    ) -> Self {
        let source = Self::new(url, namespace, branch, sources_root.as_ref());
        // Strip the scheme so the checkout mirrors the remote repository path; each remainder
        // segment is appended on its own so two sources never share a directory.
        let rest = source
            .canonical_url
            .split_once("://")
            .map_or(source.canonical_url.as_str(), |(_scheme, rest)| rest);
        let mut checkout_dir = sources_root.as_ref().to_path_buf();
        for segment in rest.split('/').filter(|segment| !segment.is_empty()) {
            checkout_dir = checkout_dir.join(segment);
        }
        Self {
            checkout_dir,
            ..source
        }
    }

    /// Validates an HTTPS Git URL and short branch name before creating a source.
    ///
    /// Configuration entry points use this checked constructor while the default source keeps
    /// the infallible [`Self::from_git`] path for the compile-time constant.
    pub fn try_from_git(
        url: impl Into<String>,
        namespace: PluginNamespace,
        branch: impl AsRef<str>,
        sources_root: impl AsRef<Path>,
    ) -> Result<Self, RegistryError> {
        let url = RepositoryUrl::parse(&url.into())?;
        let branch = GitBranchName::parse(branch.as_ref())?;
        Ok(Self::from_git(
            url.as_str(),
            namespace,
            BranchName::new(branch.as_str()),
            sources_root,
        ))
    }

    /// Returns the git URL that hosts this registry source, as the user configured it.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the canonical spelling of this source's URL.
    ///
    /// This is the value a namespace binding is keyed on, so two configurations differing only in
    /// case, credentials, a default port, a trailing slash, or a `.git` suffix are one source.
    pub fn canonical_url(&self) -> &str {
        &self.canonical_url
    }

    /// Returns the namespace every plugin this source publishes is identified under.
    pub fn namespace(&self) -> &PluginNamespace {
        &self.namespace
    }

    /// Returns the branch this source tracks.
    pub fn branch(&self) -> &BranchName {
        &self.branch
    }

    /// Returns the local directory where this source is checked out.
    pub fn checkout_dir(&self) -> &Path {
        &self.checkout_dir
    }

    /// Returns the `registry/` directory inside the checkout that holds this source's entries.
    pub fn registry_dir(&self) -> PathBuf {
        self.checkout_dir.join(REGISTRY_DIRECTORY)
    }

    /// Returns the command environment to apply to this source's Git network work.
    pub fn git_env(&self) -> &GitEnv {
        &self.git_env
    }

    /// Replaces the command environment used for this source's Git network work.
    pub fn with_git_env(mut self, git_env: GitEnv) -> Self {
        self.git_env = git_env;
        self
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
                env: source.git_env().clone(),
            })?;
            git.checkout(CheckoutRequest {
                repository: &repository,
                branch: source.branch(),
            })?;
            git.pull(PullRequest {
                repository: &repository,
                branch: source.branch(),
                env: source.git_env().clone(),
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
                env: source.git_env().clone(),
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
            PluginNamespace::official(),
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
            PluginNamespace::official(),
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

    /// Verifies `from_git` derives a stable checkout directory from the URL beneath sources root.
    #[test]
    fn derives_checkout_dir_from_git_url() -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        let sources_root = temp.path().join("sources");
        let source = RegistrySource::from_git(
            "https://github.com/ora-space/marketplace",
            PluginNamespace::official(),
            BranchName::new("main"),
            &sources_root,
        );

        let expected_checkout = sources_root
            .join("github.com")
            .join("ora-space")
            .join("marketplace");
        assert_eq!(
            (
                source.checkout_dir(),
                source.registry_dir(),
                source.url(),
                source.branch().as_str(),
            ),
            (
                expected_checkout.as_path(),
                expected_checkout.join("registry"),
                "https://github.com/ora-space/marketplace",
                "main",
            ),
        );
        Ok(())
    }

    /// Verifies equivalent spellings of one repository share a canonical URL and one checkout,
    /// while the configured spelling is preserved for display and for the Git remote itself.
    ///
    /// The checkout is the cheap half of this: sharing it avoids cloning the same repository
    /// twice. The canonical URL is the half that matters, because it is what a namespace binding
    /// is keyed on, and a binding that split across spellings would detach installed plugins from
    /// their source.
    #[test]
    fn canonicalizes_equivalent_urls_onto_one_checkout() -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        let sources_root = temp.path().join("sources");
        let spellings = [
            "https://github.com/ora-space/marketplace",
            "https://GitHub.com/Ora-Space/Marketplace.git",
            "https://github.com:443/ora-space/marketplace/",
        ];

        let sources = spellings.map(|url| {
            RegistrySource::from_git(
                url,
                PluginNamespace::official(),
                BranchName::new("main"),
                &sources_root,
            )
        });

        let expected_canonical = "https://github.com/ora-space/marketplace";
        let expected_checkout = sources_root
            .join("github.com")
            .join("ora-space")
            .join("marketplace");
        assert_eq!(
            sources
                .iter()
                .map(|source| (source.canonical_url(), source.checkout_dir(), source.url()))
                .collect::<Vec<_>>(),
            spellings
                .iter()
                .map(|url| (expected_canonical, expected_checkout.as_path(), *url))
                .collect::<Vec<_>>(),
        );
        Ok(())
    }

    /// Verifies checked source construction normalizes a valid HTTPS URL and branch.
    #[test]
    fn validates_checked_git_source() -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        let namespace = PluginNamespace::parse("acme-plugins.a1b2c3d4").expect("namespace");
        let source = RegistrySource::try_from_git(
            "https://github.com/ora-space/marketplace",
            namespace.clone(),
            "main",
            temp.path(),
        )?;

        assert_eq!(
            (
                source.url(),
                source.branch().as_str(),
                source.namespace(),
                source.checkout_dir().starts_with(temp.path()),
            ),
            (
                "https://github.com/ora-space/marketplace",
                "main",
                &namespace,
                true,
            ),
        );
        Ok(())
    }

    /// Verifies checked source construction rejects HTTP and malformed Git branch names.
    #[test]
    fn rejects_invalid_checked_git_source() -> Result<(), Box<dyn std::error::Error>> {
        assert!(
            RegistrySource::try_from_git(
                "http://github.com/example/marketplace",
                PluginNamespace::official(),
                "main",
                tempfile::tempdir()?.path(),
            )
            .is_err()
        );
        assert!(
            RegistrySource::try_from_git(
                "https://github.com/example/marketplace",
                PluginNamespace::official(),
                "feature api",
                tempfile::tempdir()?.path(),
            )
            .is_err()
        );
        Ok(())
    }
}
