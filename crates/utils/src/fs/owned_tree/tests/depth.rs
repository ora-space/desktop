//! The depth bound: whoever may create directories in the tree must not choose how many
//! descriptors the walk holds.

use super::{OwnTreeError, foreign_group, own_ids, own_tree};
use crate::fs::MAX_OWN_TREE_DEPTH;
use pretty_assertions::assert_eq;
use rustix::fs::{AtFlags, CWD, Mode, OFlags, fstat, mkdirat, openat, renameat, unlinkat};
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
use std::ffi::CStr;
use std::fs;
use std::path::Path;

/// Descriptors other threads of the test process may open while one walk is measured.
const OTHER_TESTS_SLACK: usize = 64;

/// Builds `root/d/d/…/d/leaf`, `depth` directories deep, holding at most two descriptors at a
/// time and never a path of several thousand components.
fn chain(root: &Path, depth: usize) -> rustix::io::Result<()> {
    let flags = OFlags::DIRECTORY | OFlags::CLOEXEC;
    let mut current = openat(CWD, root, flags, Mode::empty())?;
    for _ in 0..depth {
        mkdirat(&current, c"d", Mode::from_raw_mode(0o755))?;
        current = openat(&current, c"d", flags, Mode::empty())?;
    }
    let create = OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC;
    openat(&current, c"leaf", create, Mode::from_raw_mode(0o644)).map(drop)
}

/// Removes a chain level by level from the top, with constant descriptors and no long paths:
/// `root/d/d` moves up to replace `root/d`, which is then empty.
fn remove_chain(root: &Path) -> rustix::io::Result<()> {
    let root = openat(
        CWD,
        root,
        OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    loop {
        let top = match openat(
            &root,
            c"d",
            OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(top) => top,
            Err(rustix::io::Errno::NOENT) => return Ok(()),
            Err(error) => return Err(error),
        };
        match renameat(&top, c"d", &root, c"next") {
            Ok(()) => {}
            // The bottom level: only the leaf is left in it.
            Err(rustix::io::Errno::NOENT) => {
                unlinkat(&top, c"leaf", AtFlags::empty())?;
                return unlinkat(&root, c"d", AtFlags::REMOVEDIR);
            }
            Err(error) => return Err(error),
        }
        unlinkat(&root, c"d", AtFlags::REMOVEDIR)?;
        renameat(&root, c"next", &root, c"d")?;
    }
}

/// How many descriptors this process holds right now.
fn open_descriptors() -> usize {
    fs::read_dir("/proc/self/fd").map_or(0, Iterator::count)
}

/// Makes sure this process may hold `needed` descriptors, raising its soft `RLIMIT_NOFILE` up to
/// the hard limit when it is lower (a stock desktop shell has a soft limit of 1024). Only a hard
/// limit below `needed` fails, since nothing unprivileged can raise that.
fn allow_descriptors(needed: u64) {
    let limit = getrlimit(Resource::Nofile);
    if limit.current.is_none_or(|soft| soft >= needed) {
        return;
    }
    assert!(
        limit.maximum.is_none_or(|hard| hard >= needed),
        "the hard descriptor limit {:?} is below the {needed} a walk at the depth bound needs",
        limit.maximum
    );
    setrlimit(
        Resource::Nofile,
        Rlimit {
            current: limit.maximum,
            maximum: limit.maximum,
        },
    )
    .unwrap_or_else(|error| panic!("raising the soft descriptor limit failed: {error}"));
}

/// The group of the directory `level` levels down a chain, read through descriptors so no path
/// of several thousand components is ever built.
fn group_at(root: &Path, level: usize) -> rustix::io::Result<u32> {
    let flags = OFlags::DIRECTORY | OFlags::CLOEXEC;
    let mut current = openat(CWD, root, flags, Mode::empty())?;
    for _ in 0..level {
        current = openat(&current, c"d", flags, Mode::empty())?;
    }
    Ok(fstat(&current)?.st_gid)
}

/// A tree exactly [`MAX_OWN_TREE_DEPTH`] directories deep (with a file in the deepest one) is
/// handed over entirely; one level more is refused with [`OwnTreeError::TooDeep`] before the walk
/// opens or re-owns it, and a much deeper chain never makes the walk hold more than the bounded
/// number of descriptors.
#[test]
fn the_walk_refuses_trees_deeper_than_the_bound() -> Result<(), Box<dyn std::error::Error>> {
    // A walk at the bound holds about MAX_OWN_TREE_DEPTH descriptors itself.
    allow_descriptors(MAX_OWN_TREE_DEPTH as u64 + 1024);
    let (at_limit, beyond) = (tempfile::tempdir()?, tempfile::tempdir()?);
    let (uid, gid) = own_ids(at_limit.path());
    // With a foreign group the walk really re-owns, so "the refused level is left as it was" is
    // observable; with the opt-out the bound itself is still exercised.
    let target = foreign_group(gid, "the refused level is not observably left alone");
    chain(at_limit.path(), MAX_OWN_TREE_DEPTH)?;
    chain(beyond.path(), 2 * MAX_OWN_TREE_DEPTH)?;

    let at_limit_result = own_tree(at_limit.path(), uid, target.unwrap_or(gid), |_: &CStr| {});
    let before = open_descriptors();
    let mut pinned = 0;
    let mut peak = before;
    let beyond_result = own_tree(beyond.path(), uid, target.unwrap_or(gid), |_: &CStr| {
        pinned += 1;
        // The deepest point is the end of a bounded walk; sample there, and sparsely beyond it
        // in case the bound is gone.
        if pinned + 8 >= MAX_OWN_TREE_DEPTH
            && (pinned <= MAX_OWN_TREE_DEPTH + 8 || pinned % 256 == 0)
        {
            peak = peak.max(open_descriptors());
        }
    });
    let groups = (
        group_at(beyond.path(), MAX_OWN_TREE_DEPTH)?,
        group_at(beyond.path(), MAX_OWN_TREE_DEPTH + 1)?,
    );
    remove_chain(at_limit.path())?;
    remove_chain(beyond.path())?;

    // The root, every level and the leaf; nothing when the walk could only use its own ids.
    let expected_changes = if target.is_some() {
        MAX_OWN_TREE_DEPTH + 2
    } else {
        0
    };
    assert_eq!(at_limit_result?, expected_changes);
    assert!(
        matches!(beyond_result, Err(OwnTreeError::TooDeep)),
        "a chain deeper than the bound was not refused: {beyond_result:?}"
    );
    // The deepest level the walk may enter was re-owned; the refused one below it was not.
    assert_eq!(groups, (target.unwrap_or(gid), gid));
    // The levels from the root to the bound, the refused level's pin and a listing's two.
    assert!(
        peak - before <= MAX_OWN_TREE_DEPTH + 3 + OTHER_TESTS_SLACK,
        "the walk held {} descriptors",
        peak - before
    );
    assert_eq!(pinned, MAX_OWN_TREE_DEPTH + 1);
    Ok(())
}
