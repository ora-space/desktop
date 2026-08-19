//! Shared fixtures for integration tests that exercise real Git repositories.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::{Builder, TempDir};

/// Returns the result type used by the shared Git test fixture.
pub type GitTestResult<T> = Result<T, String>;

/// Owns an isolated temporary repository and the linked-worktree paths used by a test.
///
/// The fixture keeps Git's global configuration inside the temporary directory so tests do
/// not depend on, or mutate, the developer's Git configuration. The `TempDir` owns the whole
/// sandbox and removes it automatically after the fixture is dropped.
#[derive(Debug)]
pub struct GitTestScaffold {
    temp_dir: TempDir,
    repo_path: PathBuf,
    worktrees_root: PathBuf,
    global_config_path: PathBuf,
}

impl GitTestScaffold {
    /// Creates an isolated repository with a deterministic local Git identity and environment.
    pub fn new(test_name: &str) -> GitTestResult<Self> {
        let temp_dir = Builder::new()
            .prefix(test_name)
            .tempdir()
            .map_err(|error| format!("create Git test sandbox failed: {error}"))?;
        let repo_path = temp_dir.path().join("repo");
        let worktrees_root = temp_dir.path().join("worktrees");
        let global_config_path = temp_dir.path().join("gitconfig");

        fs::create_dir_all(&repo_path)
            .map_err(|error| format!("create Git test repository failed: {error}"))?;
        fs::create_dir_all(&worktrees_root)
            .map_err(|error| format!("create Git test worktree root failed: {error}"))?;
        fs::write(&global_config_path, [])
            .map_err(|error| format!("create isolated Git config failed: {error}"))?;

        let scaffold = Self {
            temp_dir,
            repo_path,
            worktrees_root,
            global_config_path,
        };

        scaffold.run_git([
            OsStr::new("init"),
            OsStr::new("--initial-branch=main"),
            OsStr::new("."),
        ])?;
        scaffold.run_git([
            OsStr::new("config"),
            OsStr::new("user.name"),
            OsStr::new("ora-test-support"),
        ])?;
        scaffold.run_git([
            OsStr::new("config"),
            OsStr::new("user.email"),
            OsStr::new("ora-test-support@example.com"),
        ])?;

        Ok(scaffold)
    }

    /// Returns the isolated repository checkout used as the main worktree.
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    /// Returns the root of the temporary sandbox owned by this fixture.
    pub fn sandbox_root(&self) -> &Path {
        self.temp_dir.path()
    }

    /// Returns the directory reserved for linked worktree checkouts.
    pub fn worktrees_root(&self) -> &Path {
        &self.worktrees_root
    }

    /// Returns the path a named linked worktree should use inside this sandbox.
    pub fn linked_worktree_path(&self, name: &str) -> PathBuf {
        self.worktrees_root.join(name)
    }

    /// Creates a linked worktree on a new branch and returns its checkout path.
    pub fn create_linked_worktree(&self, name: &str, branch_name: &str) -> GitTestResult<PathBuf> {
        let worktree_path = self.linked_worktree_path(name);
        self.run_git([
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("-b"),
            OsStr::new(branch_name),
            worktree_path.as_os_str(),
        ])?;

        Ok(worktree_path)
    }

    /// Stages all current changes and creates a commit without invoking a developer's signer.
    pub fn stage_all_and_commit(&self, message: &str) -> GitTestResult<()> {
        self.run_git([OsStr::new("add"), OsStr::new(".")])?;
        self.run_git([
            OsStr::new("commit"),
            OsStr::new("--no-gpg-sign"),
            OsStr::new("-m"),
            OsStr::new(message),
        ])?;
        Ok(())
    }

    /// Writes a UTF-8 fixture file relative to a supplied checkout root.
    pub fn write_file(
        &self,
        root: impl AsRef<Path>,
        relative_path: impl AsRef<Path>,
        contents: &str,
    ) -> GitTestResult<PathBuf> {
        let path = root.as_ref().join(relative_path.as_ref());

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create fixture parent directories failed: {error}"))?;
        }

        fs::write(&path, contents)
            .map_err(|error| format!("write fixture file failed: {error}"))?;
        Ok(path)
    }

    /// Runs Git arguments in the main repository checkout.
    pub fn run_git<I, S>(&self, args: I) -> GitTestResult<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_git(&self.repo_path, &self.global_config_path, args)
    }

    /// Runs Git arguments in an arbitrary checkout owned by this fixture.
    pub fn run_git_in<I, S>(&self, cwd: impl AsRef<Path>, args: I) -> GitTestResult<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_git(cwd.as_ref(), &self.global_config_path, args)
    }
}

/// Runs Git with an isolated configuration and stable output settings.
fn run_git<I, S>(cwd: &Path, global_config_path: &Path, args: I) -> GitTestResult<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", global_config_path)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .args(args)
        .output()
        .map_err(|error| format!("spawn git failed: {error}"))?;

    if output.status.success() {
        return String::from_utf8(output.stdout)
            .map_err(|error| format!("git stdout is not valid UTF-8: {error}"));
    }

    Err(format!(
        "git failed with code {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    ))
}

#[cfg(test)]
mod tests {
    use super::GitTestScaffold;
    use pretty_assertions::assert_eq;

    /// Verifies independent fixtures use separate roots and deterministic identities.
    #[test]
    fn creates_isolated_repositories_with_local_identity() {
        let first = GitTestScaffold::new("first")
            .unwrap_or_else(|error| panic!("create first fixture failed: {error}"));
        let second = GitTestScaffold::new("second")
            .unwrap_or_else(|error| panic!("create second fixture failed: {error}"));

        assert_ne!(first.sandbox_root(), second.sandbox_root());
        assert_eq!(
            first
                .run_git(["config", "user.name"])
                .unwrap_or_else(|error| panic!("read fixture identity failed: {error}")),
            "ora-test-support\n"
        );
    }
}
