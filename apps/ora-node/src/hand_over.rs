//! Handing a checkout to the workload user: the one owner of the privileged walk's host check
//! and of how many such walks the whole Node runs at once.
//!
//! Session starts and Revision deliveries both re-own a checkout the workload user may be
//! changing while the walk runs. Two properties of that walk are Node-wide rather than per
//! checkout, so they live here instead of with either caller:
//!
//! - **Descriptors.** One walk holds up to `MAX_OWN_TREE_DEPTH + 3` descriptors, and the workload
//!   user decides how deep every checkout it can write is. Bounding concurrent walks across the
//!   whole Node — not per checkout — is what makes the documented `RLIMIT_NOFILE` minimum hold
//!   however many checkouts are deep.
//! - **Hard links.** The walk leaves a file alone while another name links it, but a writer can
//!   remove its own name between the walk pinning the inode and reading its link count, so that
//!   skip only stops links that stay in place. The guarantee is `fs.protected_hardlinks = 1`,
//!   which keeps the workload user from linking foreign files at all; it is global host state
//!   that can change after the Node started, so it is read again before every walk.

use ora_utils::fs::OwnTreeError;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, LazyLock, Mutex, PoisonError};

/// How many checkout walks the whole Node runs at once: the eight start walks one checkout can
/// have (one per unfinished session) plus the one delivery the Node prepares at a time.
pub(crate) const MAX_CONCURRENT_WALKS: usize = 9;

/// Where Linux reports whether unprivileged users may hard-link files they do not own.
pub(crate) const PROTECTED_HARDLINKS: &str = "/proc/sys/fs/protected_hardlinks";

/// The permits every production hand-over shares.
static NODE_WALKS: LazyLock<Arc<WalkPermits>> =
    LazyLock::new(|| Arc::new(WalkPermits::new(MAX_CONCURRENT_WALKS)));

/// Why a checkout was not handed to the workload user.
#[derive(Debug, thiserror::Error)]
pub(crate) enum HandOverError {
    /// The checkout nests directories deeper than the walk descends.
    #[error(
        "checkout is nested deeper than {} directory levels",
        ora_utils::fs::MAX_OWN_TREE_DEPTH
    )]
    TooDeep,
    /// The host no longer stops the workload user from hard-linking foreign files.
    #[error(transparent)]
    HardlinksUnprotected(io::Error),
    /// The walk could not open, list or re-own an entry.
    #[error(transparent)]
    Io(io::Error),
}

/// Hands checkouts to the workload user under the Node-wide walk bound, after confirming the
/// host still protects hard links.
#[derive(Clone)]
pub struct CheckoutHandOver {
    protected_hardlinks: PathBuf,
    permits: Arc<WalkPermits>,
}

impl CheckoutHandOver {
    /// A hand-over that reads `fs.protected_hardlinks` from `protected_hardlinks` and shares the
    /// Node-wide walk permits; production passes [`PROTECTED_HARDLINKS`], tests a stand-in file.
    pub fn new(protected_hardlinks: impl Into<PathBuf>) -> Self {
        Self {
            protected_hardlinks: protected_hardlinks.into(),
            permits: NODE_WALKS.clone(),
        }
    }

    /// Gives `checkout` to `uid`:`gid` without following links, waiting for a walk permit first.
    pub(crate) fn hand_over(
        &self,
        checkout: &Path,
        uid: u32,
        gid: u32,
    ) -> Result<usize, HandOverError> {
        self.walk(|| ora_utils::fs::own_tree_no_follow(checkout, uid, gid))
    }

    /// Runs one walk under a permit, once the host check passed; `walk` is the seam tests use
    /// to observe how many walks overlap.
    fn walk<W: FnOnce() -> Result<usize, OwnTreeError>>(
        &self,
        walk: W,
    ) -> Result<usize, HandOverError> {
        let _permit = self.permits.acquire();
        // Read under the permit, right before the walk, so a value changed while this walk
        // waited is still seen.
        require_protected_hardlinks(&self.protected_hardlinks)
            .map_err(HandOverError::HardlinksUnprotected)?;
        walk().map_err(|error| match error {
            OwnTreeError::TooDeep => HandOverError::TooDeep,
            OwnTreeError::Io(error) => HandOverError::Io(error),
        })
    }
}

impl std::fmt::Debug for CheckoutHandOver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CheckoutHandOver")
            .field("protected_hardlinks", &self.protected_hardlinks)
            .finish_non_exhaustive()
    }
}

impl PartialEq for CheckoutHandOver {
    /// Two hand-overs are the same when they read the same setting and share the same permits.
    fn eq(&self, other: &Self) -> bool {
        self.protected_hardlinks == other.protected_hardlinks
            && Arc::ptr_eq(&self.permits, &other.permits)
    }
}

impl Eq for CheckoutHandOver {}

/// Fails unless `sysctl` reads `1`, naming the setting and where it was read.
///
/// The kernel default is 0 even where systemd's defaults set it to 1, and the setting is not
/// namespaced, so a container sees whatever its host has now.
pub(crate) fn require_protected_hardlinks(sysctl: &Path) -> io::Result<()> {
    let refused = |found: &str| {
        io::Error::other(format!(
            "process.workload_uid requires the sysctl fs.protected_hardlinks = 1 ({}), but {found}",
            sysctl.display()
        ))
    };
    match std::fs::read_to_string(sysctl) {
        Ok(value) if value.trim() == "1" => Ok(()),
        Ok(value) => Err(refused(&format!("it reads {:?}", value.trim()))),
        Err(error) => Err(refused(&format!("it could not be read: {error}"))),
    }
}

/// A counting semaphore for blocking walks; the walks run on blocking threads, never on the
/// async workers.
struct WalkPermits {
    free: Mutex<usize>,
    released: Condvar,
}

impl WalkPermits {
    fn new(permits: usize) -> Self {
        Self {
            free: Mutex::new(permits),
            released: Condvar::new(),
        }
    }

    /// Waits until a permit is free and holds it until the returned guard drops.
    fn acquire(&self) -> WalkPermit<'_> {
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        while *free == 0 {
            free = self
                .released
                .wait(free)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *free -= 1;
        WalkPermit { permits: self }
    }
}

/// One held walk permit, returned when dropped.
struct WalkPermit<'a> {
    permits: &'a WalkPermits,
}

impl Drop for WalkPermit<'_> {
    fn drop(&mut self) {
        *self
            .permits
            .free
            .lock()
            .unwrap_or_else(PoisonError::into_inner) += 1;
        self.permits.released.notify_one();
    }
}

#[cfg(test)]
mod tests;
