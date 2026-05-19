use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Creates an isolated on-disk Git sandbox for integration tests.
#[allow(dead_code)]
pub struct TestScaffold {
    tmp_root: PathBuf,
    sandbox_root: PathBuf,
    repo_path: PathBuf,
    worktrees_root: PathBuf,
}

#[allow(dead_code)]
impl TestScaffold {
    /// Creates a unique sandbox repository under `.tmp` and initializes Git identity for commits.
    pub fn new(test_name: &str) -> Result<Self, String> {
        let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let tmp_root = project_root.join(".tmp");
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("system time error: {e}"))?
            .as_nanos();
        let unique = format!("{test_name}-{}-{ts}", std::process::id());
        let sandbox_root = tmp_root.join(unique);
        let repo_path = sandbox_root.join("repo");
        let worktrees_root = sandbox_root.join("worktrees");

        fs::create_dir_all(&repo_path).map_err(|e| format!("create sandbox failed: {e}"))?;
        fs::create_dir_all(&worktrees_root)
            .map_err(|e| format!("create worktrees directory failed: {e}"))?;

        run_git(&repo_path, ["init", "--initial-branch=main", "."])?;
        run_git(&repo_path, ["config", "user.name", "gitlancer-test"])?;
        run_git(
            &repo_path,
            ["config", "user.email", "gitlancer-test@example.com"],
        )?;

        Ok(Self {
            tmp_root,
            sandbox_root,
            repo_path,
            worktrees_root,
        })
    }

    /// Returns the initialized repository absolute path.
    pub fn repo_path(&self) -> &Path {
        &self.repo_path
    }

    /// Returns the root `.tmp` path used by test sandboxes.
    pub fn tmp_root(&self) -> &Path {
        &self.tmp_root
    }

    /// Returns the sandbox root so tests can inspect all generated fixtures for one case together.
    pub fn sandbox_root(&self) -> &Path {
        &self.sandbox_root
    }

    /// Returns the directory reserved for linked worktree checkouts in this sandbox.
    pub fn worktrees_root(&self) -> &Path {
        &self.worktrees_root
    }

    /// Returns the filesystem path a named linked worktree should use inside this sandbox.
    pub fn linked_worktree_path(&self, name: &str) -> PathBuf {
        self.worktrees_root.join(name)
    }

    /// Creates a linked worktree so multi-worktree integration tests can stay focused on behavior.
    pub fn create_linked_worktree(&self, name: &str, branch_name: &str) -> Result<PathBuf, String> {
        let worktree_path = self.linked_worktree_path(name);
        let path_arg = worktree_path.to_string_lossy().into_owned();
        run_git(
            &self.repo_path,
            ["worktree", "add", "-b", branch_name, &path_arg],
        )?;

        Ok(worktree_path)
    }

    /// Stages all current changes and creates one commit so integration tests can bootstrap repository history quickly.
    pub fn stage_all_and_commit(&self, message: &str) -> Result<(), String> {
        run_git(&self.repo_path, ["add", "."])?;
        run_git(&self.repo_path, ["commit", "--no-gpg-sign", "-m", message])?;
        Ok(())
    }

    /// Writes a UTF-8 file relative to one root path so tests can set up repository state tersely.
    pub fn write_file(
        &self,
        root: impl AsRef<Path>,
        relative_path: impl AsRef<Path>,
        contents: &str,
    ) -> Result<PathBuf, String> {
        let path = root.as_ref().join(relative_path.as_ref());

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("create parent directories failed: {e}"))?;
        }

        fs::write(&path, contents).map_err(|e| format!("write file failed: {e}"))?;
        Ok(path)
    }

    /// Runs raw Git arguments inside this sandbox repository.
    pub fn run_git<I, S>(&self, args: I) -> Result<String, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_git(&self.repo_path, args)
    }

    /// Runs raw Git arguments inside an arbitrary directory so linked worktree tests can target the correct checkout.
    pub fn run_git_in<I, S>(&self, cwd: impl AsRef<Path>, args: I) -> Result<String, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        run_git(cwd.as_ref(), args)
    }
}

impl Drop for TestScaffold {
    /// Deletes only this sandbox so concurrently running tests do not remove each other's fixtures.
    fn drop(&mut self) {
        if self.sandbox_root.exists() {
            let _ = fs::remove_dir_all(&self.sandbox_root);
        }
    }
}

/// Runs a Git command in a target directory and returns stdout when successful.
fn run_git<I, S>(cwd: &Path, args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LANG", "C")
        .env("GIT_PAGER", "cat")
        .args(args)
        .output()
        .map_err(|e| format!("spawn git failed: {e}"))?;

    if output.status.success() {
        return String::from_utf8(output.stdout).map_err(|e| format!("stdout utf8 error: {e}"));
    }

    Err(format!(
        "git failed with code {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    ))
}
