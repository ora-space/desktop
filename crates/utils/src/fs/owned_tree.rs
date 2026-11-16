//! Hands a directory tree to one owner without following links or re-resolving a path.
//!
//! A privileged caller re-owning a tree that less privileged processes may change at the same
//! time cannot trust any path below the root: between inspecting `dir/file` and changing it, a
//! writer can swap `dir` for a link to somewhere else, or `file` for a hard link to an inode that
//! lives outside the tree, and a path-based `lchown("dir/file")` acts on whatever the name means
//! by then. This walk therefore resolves every name exactly once: it pins each entry with
//! `O_PATH | O_NOFOLLOW` relative to the descriptor of the directory that listed it, and from then
//! on inspects, re-owns and (for a directory) lists that pinned inode only through its descriptor.
//! A later swap of the name can no longer change what the walk acts on. Hard links are the one
//! case pinning cannot settle: the walk sees an inode, not where else it is named, so callers
//! whose writers could link files they do not own need the platform to refuse that link.

use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{
    AtFlags, CWD, Dir, FileType, Gid, Mode, OFlags, Stat, Uid, chownat, fstat, openat,
};
use rustix::io::Errno;
use std::ffi::{CStr, CString};
use std::io;
use std::path::Path;

/// How many directory levels below the root the walk descends before it refuses the tree.
///
/// The walk keeps one descriptor open per directory level between the root and the entry it
/// visits, and descriptors are a process-wide resource, so whoever may create directories in the
/// tree would otherwise choose how many descriptors the caller holds. The bound caps one walk at
/// `MAX_OWN_TREE_DEPTH + 3` open descriptors; no real source tree comes close to it.
pub const MAX_OWN_TREE_DEPTH: usize = 4096;

/// Why [`own_tree_no_follow`] did not hand the whole tree over.
#[derive(Debug, thiserror::Error)]
pub enum OwnTreeError {
    /// A directory lies more than [`MAX_OWN_TREE_DEPTH`] levels below the root. Nothing deeper
    /// was opened; the levels above may already have been re-owned.
    #[error("directory tree is nested deeper than {MAX_OWN_TREE_DEPTH} levels")]
    TooDeep,
    /// The root is missing or not a directory, or an entry could not be opened, listed or
    /// re-owned.
    #[error(transparent)]
    Io(#[from] io::Error),
}

impl From<Errno> for OwnTreeError {
    fn from(error: Errno) -> Self {
        Self::Io(error.into())
    }
}

impl From<OwnTreeError> for io::Error {
    /// Lets callers that only report failures keep using `io::Result`.
    fn from(error: OwnTreeError) -> Self {
        match error {
            OwnTreeError::TooDeep => io::Error::other(error),
            OwnTreeError::Io(error) => error,
        }
    }
}

/// Pins one entry without following it: a symbolic link yields a descriptor of the link itself,
/// and a FIFO or device is not opened at all, so pinning never blocks or acts on a target.
const PINNED: OFlags = OFlags::PATH.union(OFlags::NOFOLLOW).union(OFlags::CLOEXEC);

/// Pins the root, which must be a directory; a link or another file type fails with `ENOTDIR`.
const ROOT: OFlags = PINNED.union(OFlags::DIRECTORY);

/// Opens a pinned directory (as `.` relative to its own descriptor) for enumeration.
const LISTED: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::CLOEXEC);

/// Gives every entry under `root` (and `root` itself) to `uid`:`gid`, returning how many entries
/// changed owner.
///
/// Safe against concurrent writers: no path below `root` is ever re-resolved. Each entry is
/// pinned with `openat(O_PATH | O_NOFOLLOW)` relative to the descriptor of the directory that
/// listed it, inspected with `fstat` and re-owned with `fchownat(AT_EMPTY_PATH)` on that pinned
/// descriptor; a directory is then listed by opening `.` relative to it. A symbolic link is
/// pinned, and re-owned, as the link itself and never followed, so neither a planted link nor a
/// directory swapped for one mid-walk can make a privileged caller change anything outside the
/// tree, and a name swapped after its entry was pinned changes nothing the walk does.
///
/// A non-directory with more than one hard link that is not already owned by `uid`:`gid` is left
/// as it is: its other names may lie outside the tree, so re-owning the inode could hand an
/// outside file to the new owner. Such a file keeps its owner and whatever access its mode grants;
/// once only the tree names an inode, it is re-owned like any other entry. This only stops links
/// that stay in place: the link count is read after pinning, so a writer that links an outside
/// file into the tree and removes that name right after the walk pinned it leaves a count of one,
/// and the outside inode is re-owned. No re-check closes that race. Callers whose writers could
/// hard-link files they do not own must rely on the platform refusing the link
/// (`fs.protected_hardlinks = 1` on Linux); that is the guarantee, the skip is defence in depth.
///
/// An entry that disappears between being listed and being pinned (`ENOENT`) is skipped: it owns
/// nothing that is still reachable from the tree. The root itself must exist and be a directory
/// that is not a link. A directory more than [`MAX_OWN_TREE_DEPTH`] levels below the root fails
/// with [`OwnTreeError::TooDeep`]; every other failure (an entry the caller may not re-own, a
/// directory it may not list) is reported. Entries already owned by `uid`:`gid` are left
/// untouched, so a tree that is already right costs only a walk. Permission bits are not changed:
/// the new owner gets whatever access the owner bits already grant.
pub fn own_tree_no_follow(root: &Path, uid: u32, gid: u32) -> Result<usize, OwnTreeError> {
    own_tree(root, uid, gid, |_| {})
}

/// The walk behind [`own_tree_no_follow`], calling `pinned` with each entry's name after the
/// entry was pinned and before it is inspected, re-owned or listed.
///
/// The hook is the seam tests use to change the tree at exactly the step a concurrent writer
/// would; production passes a no-op, which monomorphizes away.
fn own_tree<F: FnMut(&CStr)>(
    root: &Path,
    uid: u32,
    gid: u32,
    mut pinned: F,
) -> Result<usize, OwnTreeError> {
    let mut walk = Walk {
        owner: (uid, gid),
        stack: Vec::new(),
        changed: 0,
    };
    // The root is the caller's own path; only its final component is refused as a link, exactly
    // like every entry below it. A missing root is an error, not an empty tree. The root then
    // goes through the same steps as every entry, so they share one re-own and one listing.
    walk.visit(openat(CWD, root, ROOT, Mode::empty())?)?;
    while let Some(listing) = walk.stack.last_mut() {
        let Some(name) = listing.names.next() else {
            walk.stack.pop();
            continue;
        };
        let entry = match openat(&listing.directory, &name, PINNED, Mode::empty()) {
            Ok(entry) => entry,
            // Removed since it was listed; nothing is left to re-own.
            Err(Errno::NOENT) => continue,
            Err(error) => return Err(error.into()),
        };
        pinned(&name);
        walk.visit(entry)?;
    }
    Ok(walk.changed)
}

/// The walk's progress: the directories between the root and the entry being visited.
struct Walk {
    owner: (u32, u32),
    /// One listing per directory level; its length is the depth of the next entry visited.
    stack: Vec<Listing>,
    changed: usize,
}

impl Walk {
    /// Inspects, re-owns and, for a directory, lists one pinned entry.
    fn visit(&mut self, entry: OwnedFd) -> Result<(), OwnTreeError> {
        let stat = fstat(&entry)?;
        let directory = is_directory(&stat);
        // Checked before the directory is re-owned or listed, so a refused level holds no
        // descriptor beyond the pinned one and is left exactly as it was.
        if directory && self.stack.len() > MAX_OWN_TREE_DEPTH {
            return Err(OwnTreeError::TooDeep);
        }
        let (uid, gid) = self.owner;
        let owned = stat.st_uid == uid && stat.st_gid == gid;
        // A directory cannot be hard-linked, and its link count only counts its subdirectories.
        let linked_elsewhere = !directory && stat.st_nlink > 1;
        if !owned && !linked_elsewhere {
            // `AT_EMPTY_PATH` changes the pinned inode itself, a link as a link; nothing is
            // looked up by name, so `AT_SYMLINK_NOFOLLOW` only restates that.
            chownat(
                &entry,
                c"",
                Some(Uid::from_raw(uid)),
                Some(Gid::from_raw(gid)),
                AtFlags::EMPTY_PATH | AtFlags::SYMLINK_NOFOLLOW,
            )?;
            self.changed += 1;
        }
        if directory {
            self.stack.push(Listing::open(&entry)?);
        }
        Ok(())
    }
}

/// Reads the file type from a raw `st_mode`, whose integer width differs between platforms.
fn is_directory(stat: &Stat) -> bool {
    #[allow(
        clippy::unnecessary_cast,
        reason = "st_mode is narrower than RawMode on some targets"
    )]
    let mode = stat.st_mode as rustix::fs::RawMode;
    FileType::from_raw_mode(mode).is_dir()
}

/// One directory being walked: its descriptor and the names it listed that are still unvisited.
struct Listing {
    directory: OwnedFd,
    names: std::vec::IntoIter<CString>,
}

impl Listing {
    /// Opens a pinned directory through its own descriptor and lists it before any of its
    /// entries is touched.
    ///
    /// Opening `.` relative to the pinned descriptor reaches exactly the pinned inode, whatever
    /// its name means by now. Names are collected first so the entries below can be pinned
    /// relative to `directory` while it stays open; a directory removed meanwhile simply lists
    /// nothing more.
    fn open(entry: &OwnedFd) -> io::Result<Self> {
        let directory = openat(entry, c".", LISTED, Mode::empty())?;
        let mut names = Vec::new();
        for item in Dir::read_from(directory.as_fd())? {
            let item = item?;
            let name = item.file_name();
            if name != c"." && name != c".." {
                names.push(name.to_owned());
            }
        }
        Ok(Self {
            directory,
            names: names.into_iter(),
        })
    }
}

#[cfg(test)]
mod tests;
