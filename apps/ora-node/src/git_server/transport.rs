//! Private bare repositories every Node-identity Git command runs in.
//!
//! They live under `<repository_root>/.ora-transport/`, a Node-owned `0711` directory beside the
//! checkouts, rather than under the Node home: a clone, a restore and an `ora-git fetch` hand the
//! fetched objects to the workload user, whose Git must reach the transport repository by path,
//! and the Node home is closed to it. Each repository starts `0700` and Node-owned, and contains
//! no credentials at any time: the access material is only ever on a command line. A repository
//! is removed when its work ends, and the whole root is cleared when the Node starts.
use gitlancer::{GitCommand, GitEnv, GitIntent, GitRunner};
use ora_node_protocol::{CommitId, ExecutionId};
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Directory under the clone repository root that holds the transport repositories.
pub(crate) const TRANSPORT_ROOT: &str = ".ora-transport";

/// One transport repository, removed when dropped.
#[derive(Debug)]
pub(crate) struct TransportRepository {
    path: PathBuf,
}

impl TransportRepository {
    /// Creates an empty bare repository for one piece of work, discarding what an interrupted
    /// earlier attempt of the same work left. `kind` separates the users of one execution ID.
    pub(crate) fn create<R: GitRunner>(
        runner: &R,
        repository_root: &Path,
        kind: &str,
        execution: &ExecutionId,
    ) -> io::Result<Self> {
        let root = repository_root.join(TRANSPORT_ROOT);
        let mut builder = fs::DirBuilder::new();
        builder.mode(/*mode*/ 0o711);
        match builder.create(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        // The umask may have narrowed the mode; the workload user must be able to traverse.
        fs::set_permissions(&root, fs::Permissions::from_mode(/*mode*/ 0o711))?;
        let path = Self::location(repository_root, kind, execution);
        remove(&path)?;
        fs::DirBuilder::new().mode(/*mode*/ 0o700).create(&path)?;
        let repository = Self { path };
        repository.git(runner, &["init", "--bare", "--quiet", "--template="])?;
        // Written by the Node into its own configuration, so even a command that forgets the
        // per-command setting never enumerates a borrowed checkout's refs.
        repository.git(runner, &["config", "core.alternateRefsCommand", "true"])?;
        Ok(repository)
    }

    /// Where the repository of `kind` work for `execution` lives. The name is a digest because
    /// execution IDs are opaque protocol text, never a path component.
    pub(crate) fn location(repository_root: &Path, kind: &str, execution: &ExecutionId) -> PathBuf {
        repository_root.join(TRANSPORT_ROOT).join(format!(
            "{kind}-{}",
            ora_utils::hash::sha256_hex(execution.as_str().as_bytes())
        ))
    }

    /// Keeps the repository on disk past this handle, for work that continues in later steps;
    /// whoever finishes that work removes it with [`remove_transport`].
    pub(crate) fn keep(self) -> PathBuf {
        let path = self.path.clone();
        std::mem::forget(self);
        path
    }

    /// The repository directory, the `--git-dir` of its commands.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Borrows the objects of the checkout whose Git directory is `git_dir` as read-only data.
    ///
    /// A checkout that borrows objects itself (`objects/info/alternates`) or shares a common
    /// directory is refused: Git follows alternates transitively, so the Node would read object
    /// stores the Agent chose.
    pub(crate) fn borrow_objects(&self, git_dir: &Path) -> io::Result<()> {
        let metadata = fs::symlink_metadata(git_dir)?;
        let objects = git_dir.join("objects");
        if !metadata.is_dir()
            || fs::symlink_metadata(git_dir.join("commondir")).is_ok()
            || fs::symlink_metadata(objects.join("info").join("alternates")).is_ok()
            || !fs::symlink_metadata(&objects)?.is_dir()
        {
            return Err(io::Error::other(
                "checkout object store is shared or not a plain directory",
            ));
        }
        let objects = objects
            .to_str()
            .ok_or_else(|| io::Error::other("non-UTF-8 checkout path"))?;
        let info = self.path.join("objects").join("info");
        fs::create_dir_all(&info)?;
        fs::write(info.join("alternates"), format!("{objects}\n"))
    }

    /// Points Node-owned negotiation refs at the commits the checkout is known to have, so the
    /// Git Server sends only what is missing. Commits the store lacks are skipped.
    pub(crate) fn negotiate_from<R: GitRunner>(
        &self,
        runner: &R,
        commits: &[&CommitId],
    ) -> io::Result<()> {
        for (index, commit) in commits.iter().enumerate() {
            let object = format!("{}^{{commit}}", commit.as_str());
            if self.git(runner, &["cat-file", "-e", &object]).is_ok() {
                let name = format!("refs/ora/have/{index}");
                self.git(runner, &["update-ref", &name, commit.as_str()])?;
            }
        }
        Ok(())
    }

    /// Resolves `name` to a commit in this repository.
    pub(crate) fn commit_of<R: GitRunner>(
        &self,
        runner: &R,
        name: &str,
    ) -> io::Result<Option<CommitId>> {
        let peeled = format!("{name}^{{commit}}");
        Ok(self
            .git(runner, &["rev-parse", "--verify", "--quiet", &peeled])
            .ok()
            .map(CommitId::new))
    }

    /// Lists this repository's refs under `prefix` as `(ref, commit)` pairs.
    pub(crate) fn refs<R: GitRunner>(
        &self,
        runner: &R,
        prefix: &str,
    ) -> io::Result<Vec<(String, CommitId)>> {
        let listed = self.git(
            runner,
            &["for-each-ref", "--format=%(refname) %(objectname)", prefix],
        )?;
        Ok(listed
            .lines()
            .filter_map(|line| {
                let (name, commit) = line.split_once(' ')?;
                Some((name.to_owned(), CommitId::new(commit)))
            })
            .collect())
    }

    /// Whether `ancestor` is an ancestor of `descendant` in this repository's objects.
    pub(crate) fn is_ancestor<R: GitRunner>(
        &self,
        runner: &R,
        ancestor: &CommitId,
        descendant: &CommitId,
    ) -> bool {
        self.git(
            runner,
            &[
                "merge-base",
                "--is-ancestor",
                ancestor.as_str(),
                descendant.as_str(),
            ],
        )
        .is_ok()
    }

    /// Gives the repository to the workload user, whose Git reads the fetched objects from it.
    pub(crate) fn hand_over(&self, owner: Option<u32>) -> io::Result<()> {
        if let Some(owner) = owner {
            ora_utils::fs::own_tree_no_follow(&self.path, owner, owner)?;
        }
        Ok(())
    }

    /// Runs one local, credential-free Git command as the Node in this repository.
    fn git<R: GitRunner>(&self, runner: &R, args: &[&str]) -> io::Result<String> {
        let git_dir = self
            .path
            .to_str()
            .ok_or_else(|| io::Error::other("non-UTF-8 transport path"))?;
        let mut full = vec![
            format!("--git-dir={git_dir}"),
            // After a hand-over the workload user owns the repository; it holds no credentials
            // and the Node only reads it, so Git's ownership check has nothing to protect.
            "-c".to_owned(),
            format!("safe.directory={git_dir}"),
            "-c".to_owned(),
            "core.hooksPath=/dev/null".to_owned(),
            "-c".to_owned(),
            "core.alternateRefsCommand=true".to_owned(),
        ];
        full.extend(args.iter().map(|arg| (*arg).to_owned()));
        let env = GitEnv::default()
            .with_variable("HOME", git_dir)
            .with_variable("GIT_CONFIG_NOSYSTEM", "1")
            .with_variable("GIT_CONFIG_GLOBAL", "/dev/null");
        let command = GitCommand::new(self.path.clone(), full, env, GitIntent::Mutating);
        let output = runner
            .run_bounded(&command, 1 << 20, 64 * 1024)
            .map_err(|_| io::Error::other("transport repository Git failed"))?;
        if output.code != Some(0) {
            return Err(io::Error::other("transport repository Git failed"));
        }
        Ok(output.stdout.trim().to_owned())
    }
}

impl Drop for TransportRepository {
    /// A failure is only logged; the next Node start clears the root anyway.
    fn drop(&mut self) {
        if let Err(error) = remove(&self.path) {
            ora_logging::ora_warn!(error = %error, "transport repository was not removed");
        }
    }
}

/// Removes every transport repository a previous Node process left, except those in `keep`.
///
/// A clone whose workload sequence has not yet taken the snapshot out of its transport
/// repository still needs it: that half of a clone is never fetched again, and a step's Run may
/// even have outlived the Node under its guardian and still be reading it.
pub(crate) fn purge_transports(repository_root: &Path, keep: &[PathBuf]) -> io::Result<()> {
    let entries = match fs::read_dir(repository_root.join(TRANSPORT_ROOT)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let path = entry?.path();
        if !keep.contains(&path) {
            remove(&path)?;
        }
    }
    Ok(())
}

/// Removes a repository a previous step kept.
pub(crate) fn remove_transport(path: &Path) {
    if let Err(error) = remove(path) {
        ora_logging::ora_warn!(error = %error, "transport repository was not removed");
    }
}

/// Removes a directory tree that may not exist.
fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
