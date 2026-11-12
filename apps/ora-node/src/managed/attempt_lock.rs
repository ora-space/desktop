//! Attempt locks: the one source of evidence that a dispatched Git command has stopped.
//!
//! Every command with side effects owns a lock file that only the Node can open. The Node takes
//! an exclusive advisory lock on it and hands that open file to Git, whose descendants inherit it
//! too. The lock belongs to the open file, not to a process, so it stays held for as long as any
//! process of that tree is alive, whatever identity, process group or session it moved to, and
//! the kernel releases it when the last one ends. Taking the lock again therefore proves the tree
//! is gone, also after the Node itself was killed and lost every handle on its children.
//!
//! Finding and ending a leftover tree is a separate job. A clue written after the spawn names the
//! direct child so a restarted Node can signal its process group; losing the clue never changes
//! what the lock proves.
use ora_node_db::AttemptId;
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::fd::{AsRawFd, OwnedFd},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

/// What probing an attempt's lock says about the process tree that inherited it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopEvidence {
    /// The lock could be taken: no process holds the inherited file, so the command has stopped
    /// or never started. The two are not told apart; in both nothing can still write.
    Stopped,
    /// Another open file holds the lock: some process of the original tree is still alive.
    Running,
    /// The lock file is missing or could not be probed. Missing evidence is never permission.
    Unknown,
}

/// Whether the lock file must survive a machine crash together with a database record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LockDurability {
    /// The attempt is recorded in the Node database next. The file is synced first, so a record
    /// never outlives its lock file and gets mistaken for lost evidence.
    Recorded,
    /// Nothing durable refers to the attempt; an interrupted command is prepared again.
    Unrecorded,
}

/// The Node's private lock directory, shared by every thread that runs Git.
#[derive(Clone)]
pub(crate) struct AttemptLocks(Arc<Shared>);

struct Shared {
    directory: PathBuf,
    /// Locks of unrecorded commands that were still held when last probed: left by an earlier
    /// Node process, or by a command of this one whose tree outlived its cleanup. Each maps to
    /// the directory its command ran in.
    leftovers: Mutex<BTreeMap<AttemptId, PathBuf>>,
}

impl AttemptLocks {
    /// Opens the lock directory under the Node home, creating it owner-private.
    ///
    /// The home must be on a local filesystem: advisory locks on network filesystems do not
    /// reliably follow the open file.
    pub(crate) fn open(home: &Path) -> io::Result<Self> {
        let directory = home.join("git-attempts");
        match fs::DirBuilder::new()
            .mode(/*mode*/ 0o700)
            .create(&directory)
        {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        // SAFETY: geteuid only reads the process identity.
        let owner = unsafe { libc::geteuid() };
        // The workload user must not be able to open a lock file and hold an attempt Unknown.
        ora_utils::path::open_private_path(
            &directory,
            owner,
            ora_utils::path::TrustedPathKind::Directory,
        )?;
        Ok(Self(Arc::new(Shared {
            directory,
            leftovers: Mutex::new(BTreeMap::new()),
        })))
    }

    /// Creates the attempt's lock file before anything refers to the attempt.
    ///
    /// The file records the command's directory so a leftover lock can be attributed to the
    /// resource its command may still be writing.
    pub(crate) fn create(
        &self,
        attempt: &AttemptId,
        cwd: &Path,
        durability: LockDurability,
    ) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(/*write*/ true)
            .create_new(/*create_new*/ true)
            .mode(/*mode*/ 0o600)
            .open(self.lock_path(attempt))?;
        file.write_all(cwd.as_os_str().as_encoded_bytes())?;
        if durability == LockDurability::Recorded {
            file.sync_all()?;
            File::open(&self.0.directory)?.sync_all()?;
        }
        Ok(())
    }

    /// Takes the attempt's lock on a fresh open file, to be inherited by the command.
    ///
    /// The caller drops its own handle once the command is spawned, so that only the command's
    /// process tree keeps the lock held.
    pub(crate) fn hold(&self, attempt: &AttemptId) -> io::Result<Arc<OwnedFd>> {
        let file = File::open(self.lock_path(attempt))?;
        // SAFETY: flock only operates on the descriptor opened above.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Arc::new(OwnedFd::from(file)))
    }

    /// Notes which process to signal if this Node dies before the command ends.
    ///
    /// Written without syncing: a process crash keeps the page cache, and after a machine crash
    /// the command is gone as well. A failed write only costs the ability to end a leftover tree.
    pub(crate) fn note_child(&self, attempt: &AttemptId, pid: u32) {
        let clue = ora_utils::process::linux_process(pid)
            .and_then(|process| Ok((process.start_ticks, boot_id()?)))
            .and_then(|(start_ticks, boot)| {
                fs::write(
                    self.clue_path(attempt),
                    format!("{pid} {start_ticks} {boot}"),
                )
            });
        if let Err(error) = clue {
            ora_logging::ora_warn!(attempt = %attempt, error = %error, "termination clue for a Git command could not be written");
        }
    }

    /// Asks whether the attempt's process tree is gone, without waiting.
    pub(crate) fn probe(&self, attempt: &AttemptId) -> StopEvidence {
        let Ok(file) = File::open(self.lock_path(attempt)) else {
            return StopEvidence::Unknown;
        };
        // SAFETY: flock only operates on the probe's own descriptor; closing it on return gives
        // the lock back.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return StopEvidence::Stopped;
        }
        match io::Error::last_os_error().raw_os_error() {
            Some(libc::EWOULDBLOCK) => StopEvidence::Running,
            _ => StopEvidence::Unknown,
        }
    }

    /// Signals the process group the clue names, if the clue still identifies it.
    ///
    /// A live process with another start time means the pid was reused: nothing is signalled.
    /// When the pid is gone, whatever remains in that group can only be the original members,
    /// because Linux never gives a pid still in use as a group id to a new process. A clue from
    /// another boot names nothing on this one.
    pub(crate) fn terminate(&self, attempt: &AttemptId) {
        let Some((pid, start_ticks, boot)) = self.clue(attempt) else {
            return;
        };
        if boot_id().ok().as_deref() != Some(boot.as_str()) {
            return;
        }
        match ora_utils::process::linux_process(pid) {
            Ok(process) if process.start_ticks != start_ticks => return,
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return,
        }
        kill_group(pid);
    }

    /// Ends the tree the clue names, then probes until the tree is gone or `timeout` passes.
    pub(crate) async fn await_stop(&self, attempt: &AttemptId, timeout: Duration) -> StopEvidence {
        let deadline = Instant::now() + timeout;
        let mut pause = Duration::from_millis(/*millis*/ 1);
        loop {
            self.terminate(attempt);
            match self.probe(attempt) {
                StopEvidence::Running if Instant::now() < deadline => {}
                evidence => return evidence,
            }
            tokio::time::sleep(pause).await;
            pause = (pause * 2).min(Duration::from_millis(/*millis*/ 25));
        }
    }

    /// Removes the lock file and its clue once the result reached whoever needed it.
    pub(crate) fn retire(&self, attempt: &AttemptId) {
        for path in [self.clue_path(attempt), self.lock_path(attempt)] {
            if let Err(error) = fs::remove_file(&path)
                && error.kind() != io::ErrorKind::NotFound
            {
                ora_logging::ora_warn!(path = %path.display(), error = %error, "attempt lock file could not be removed");
            }
        }
        self.leftovers().remove(attempt);
    }

    /// Lists every lock file with the directory its command ran in.
    pub(crate) fn entries(&self) -> io::Result<Vec<(AttemptId, PathBuf)>> {
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.0.directory)? {
            let path = entry?.path();
            let attempt = path
                .extension()
                .is_some_and(|extension| extension == "lock")
                .then(|| path.file_stem().and_then(|stem| stem.to_str()))
                .flatten()
                .and_then(AttemptId::parse);
            if let Some(attempt) = attempt {
                let mut cwd = Vec::new();
                File::open(&path)?.read_to_end(&mut cwd)?;
                entries.push((
                    attempt,
                    PathBuf::from(String::from_utf8_lossy(&cwd).as_ref()),
                ));
            }
        }
        Ok(entries)
    }

    /// Keeps the lock of an unrecorded command whose tree is not known to be gone. Until it is,
    /// no further unrecorded command may start.
    pub(crate) fn keep_leftover(&self, attempt: AttemptId, cwd: PathBuf) {
        self.leftovers().insert(attempt, cwd);
    }

    /// Tries once more to end and retire every leftover, and returns the directories of those
    /// still running. Empty means unrecorded commands may start.
    pub(crate) fn leftover_directories(&self) -> Vec<PathBuf> {
        let leftovers: Vec<_> = self.leftovers().keys().cloned().collect();
        for attempt in leftovers {
            self.terminate(&attempt);
            if self.probe(&attempt) == StopEvidence::Stopped {
                self.retire(&attempt);
            }
        }
        self.leftovers().values().cloned().collect()
    }

    fn leftovers(&self) -> std::sync::MutexGuard<'_, BTreeMap<AttemptId, PathBuf>> {
        self.0
            .leftovers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_path(&self, attempt: &AttemptId) -> PathBuf {
        self.0.directory.join(format!("{attempt}.lock"))
    }

    fn clue_path(&self, attempt: &AttemptId) -> PathBuf {
        self.0.directory.join(format!("{attempt}.clue"))
    }

    /// Reads the clue as pid, start time and boot identity; anything malformed is no clue.
    fn clue(&self, attempt: &AttemptId) -> Option<(u32, u64, String)> {
        let text = fs::read_to_string(self.clue_path(attempt)).ok()?;
        let mut fields = text.split(' ');
        let clue = (
            fields.next()?.parse().ok()?,
            fields.next()?.parse().ok()?,
            fields.next()?.to_owned(),
        );
        fields.next().is_none().then_some(clue)
    }
}

/// Sends SIGKILL to the whole process group led by `pid`; a group that is already gone is fine.
pub(super) fn kill_group(pid: u32) {
    let Ok(group) = i32::try_from(pid) else {
        return;
    };
    // SAFETY: kill with a negative target signals a process group and touches no memory. Every
    // caller passes the pid of a group leader this Node started.
    unsafe { libc::kill(-group, libc::SIGKILL) };
}

/// Identifies the running boot, so a clue from before a reboot is never matched against the
/// unrelated processes that reuse its pids.
fn boot_id() -> io::Result<String> {
    Ok(fs::read_to_string("/proc/sys/kernel/random/boot_id")?
        .trim()
        .to_owned())
}
