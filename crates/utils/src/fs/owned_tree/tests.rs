//! The ownership tests need a group other than the caller's primary one, so the walk really
//! changes owners and "nothing outside the tree was re-owned" is observable. Without one they
//! would pass vacuously, so they fail instead. A host that genuinely has no supplementary group
//! for the test user can opt out explicitly with `ORA_TEST_ALLOW_NO_FOREIGN_GROUP=1`, which turns
//! those tests into skips that print why (run with `--nocapture` to see the note).

use super::{OwnTreeError, own_tree, own_tree_no_follow};
use pretty_assertions::assert_eq;
use std::ffi::CStr;
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

mod depth;
mod errors;
mod links;

/// The opt-out for hosts whose test user has no supplementary group. It is only read.
const ALLOW_NO_FOREIGN_GROUP: &str = "ORA_TEST_ALLOW_NO_FOREIGN_GROUP";

/// The caller's own ids, which an unprivileged walk can always "change" to.
fn own_ids(path: &Path) -> (u32, u32) {
    let metadata = fs::symlink_metadata(path).unwrap_or_else(|error| panic!("{error}"));
    (metadata.uid(), metadata.gid())
}

/// A group other than `primary` the caller may hand its files to, so `fchownat` really runs.
///
/// Root may use any group; an unprivileged caller may use any of its supplementary groups. When
/// the host offers none, this fails the test unless [`ALLOW_NO_FOREIGN_GROUP`] is `1`, in which
/// case it returns `None` and the caller skips with `note`.
fn foreign_group(primary: u32, note: &str) -> Option<u32> {
    if let Some(group) = supplementary_group(primary) {
        return Some(group);
    }
    assert!(
        std::env::var(ALLOW_NO_FOREIGN_GROUP).is_ok_and(|value| value == "1"),
        "no group other than {primary} is available, so {note}; give the test user a \
         supplementary group or set {ALLOW_NO_FOREIGN_GROUP}=1 to skip these tests"
    );
    eprintln!("{ALLOW_NO_FOREIGN_GROUP}=1: {note}");
    None
}

/// Finds a group other than `primary` that the caller may assign.
fn supplementary_group(primary: u32) -> Option<u32> {
    if is_root() {
        return Some(primary.wrapping_add(4242));
    }
    let output = std::process::Command::new("id").arg("-G").output().ok()?;
    String::from_utf8(output.stdout)
        .ok()?
        .split_whitespace()
        .filter_map(|group| group.parse::<u32>().ok())
        .find(|group| *group != primary)
}

/// Reads the effective uid through the filesystem, avoiding an unsafe libc call in tests.
fn is_root() -> bool {
    tempfile::tempdir()
        .ok()
        .and_then(|dir| fs::metadata(dir.path()).ok())
        .is_some_and(|metadata| metadata.uid() == 0)
}

/// The OS error a failed walk reports.
fn os_error(error: OwnTreeError) -> Option<i32> {
    io::Error::from(error).raw_os_error()
}

/// Without privilege the caller can only "change" to its own ids, which still exercises the
/// walk, the no-op skip and the refusal to follow links: following either link below would
/// fail (one dangles, the other leads into a directory nobody may read).
#[test]
fn walks_the_tree_without_following_links() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    fs::create_dir_all(root.path().join("a/b"))?;
    fs::write(root.path().join("a/b/file"), "x")?;
    let sealed = outside.path().join("sealed");
    fs::create_dir(&sealed)?;
    fs::write(sealed.join("secret"), "y")?;
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000))?;
    symlink(&sealed, root.path().join("a/linked-dir"))?;
    symlink(outside.path().join("missing"), root.path().join("dangling"))?;
    let (uid, gid) = own_ids(root.path());

    let result = own_tree_no_follow(root.path(), uid, gid);
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o700))?;
    assert_eq!(result?, 0);
    Ok(())
}

/// Only entries below the root are tolerated when they vanish: a missing root, a root that is a
/// link and a root that is a file are all reported, never treated as an empty tree.
#[test]
fn the_root_must_exist_as_a_directory() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let (uid, gid) = own_ids(temp.path());
    let directory = temp.path().join("directory");
    fs::create_dir(&directory)?;
    symlink(&directory, temp.path().join("link"))?;
    fs::write(temp.path().join("file"), "x")?;

    let kinds: Vec<_> = ["missing", "link", "file"]
        .into_iter()
        .map(|name| own_tree_no_follow(&temp.path().join(name), uid, gid).map_err(os_error))
        .collect();
    assert_eq!(
        kinds,
        vec![
            Err(Some(libc::ENOENT)),
            // `O_NOFOLLOW | O_PATH` pins the link itself, which `O_DIRECTORY` then refuses.
            Err(Some(libc::ENOTDIR)),
            Err(Some(libc::ENOTDIR)),
        ]
    );
    assert_eq!(own_tree_no_follow(&directory, uid, gid)?, 0);
    Ok(())
}

/// A real ownership change reaches the root, nested directories, files and links (as links),
/// and nothing a link points at.
#[test]
fn reowns_every_entry_and_links_as_themselves() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let (uid, gid) = own_ids(root.path());
    let Some(target) = foreign_group(gid, "the ownership change is not exercised") else {
        return Ok(());
    };
    fs::create_dir_all(root.path().join("a/b"))?;
    fs::write(root.path().join("a/b/file"), "x")?;
    fs::write(outside.path().join("victim"), "v")?;
    symlink(outside.path().join("victim"), root.path().join("a/link"))?;

    // root, a, a/b, a/b/file, a/link
    assert_eq!(own_tree_no_follow(root.path(), uid, target)?, 5);
    let groups: Vec<_> = ["", "a", "a/b", "a/b/file", "a/link"]
        .into_iter()
        .map(|entry| own_ids(&root.path().join(entry)).1)
        .collect();
    assert_eq!(groups, vec![target; 5]);
    assert_eq!(own_ids(&outside.path().join("victim")).1, gid);
    // Already right: only a walk.
    assert_eq!(own_tree_no_follow(root.path(), uid, target)?, 0);
    Ok(())
}

/// An entry removed between being listed and being pinned owns nothing reachable anymore, so
/// the walk skips it (`ENOENT` when pinning) instead of failing.
#[test]
fn an_entry_removed_before_it_is_pinned_is_skipped() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let (uid, gid) = own_ids(root.path());
    let Some(target) = foreign_group(gid, "the skipped re-own is not observable") else {
        return Ok(());
    };
    fs::write(root.path().join("a"), "x")?;
    fs::write(root.path().join("b"), "x")?;
    let (a, b) = (root.path().join("a"), root.path().join("b"));

    // Whichever entry is pinned first removes the other before the walk reaches it.
    let mut removed = false;
    let result = own_tree(root.path(), uid, target, |name: &CStr| {
        if !removed {
            removed = true;
            let other = if name == c"a" { &b } else { &a };
            fs::remove_file(other).unwrap_or_else(|error| panic!("{error}"));
        }
    });
    // The root and the entry pinned first; the other one no longer exists.
    assert_eq!(result?, 2);
    assert_eq!(usize::from(a.exists()) + usize::from(b.exists()), 1);
    Ok(())
}

/// Once an entry is pinned, swapping its name changes nothing the walk does: a directory
/// replaced by a file, removed, or moved aside for a link to an outside directory is still walked
/// as the inode that was pinned, and nothing behind the link is touched.
#[test]
fn a_name_swapped_after_pinning_changes_nothing_outside() -> Result<(), Box<dyn std::error::Error>>
{
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let (uid, gid) = own_ids(root.path());
    let Some(target) = foreign_group(gid, "nothing outside is observably left alone") else {
        return Ok(());
    };
    for name in ["flip", "gone", "swap"] {
        fs::create_dir(root.path().join(name))?;
        fs::write(root.path().join(name).join("file"), "x")?;
    }
    fs::write(outside.path().join("victim"), "v")?;
    let (flip, gone, swap) = (
        root.path().join("flip"),
        root.path().join("gone"),
        root.path().join("swap"),
    );
    let away = root.path().join("away");

    let result = own_tree(root.path(), uid, target, |name: &CStr| {
        let replaced = if name == c"flip" {
            fs::rename(&flip, root.path().join("flipped"))
                .and_then(|()| fs::write(&flip, "now a file"))
        } else if name == c"gone" {
            fs::remove_dir_all(&gone)
        } else if name == c"swap" {
            fs::rename(&swap, &away).and_then(|()| symlink(outside.path(), &swap))
        } else {
            Ok(())
        };
        replaced.unwrap_or_else(|error| panic!("{error}"));
    });
    result?;
    assert_eq!(own_ids(&outside.path().join("victim")).1, gid);
    assert_eq!(own_ids(outside.path()).1, gid);
    // The pinned directories were walked under their new names.
    assert_eq!(own_ids(&away.join("file")).1, target);
    assert_eq!(own_ids(&root.path().join("flipped/file")).1, target);
    Ok(())
}

/// A concurrent writer that keeps swapping a directory of the tree for a link to an outside
/// directory never makes the walk re-own anything outside the tree. This is review 2's probe:
/// the path-based walk re-owned outside files on its first run.
#[test]
fn a_concurrent_symlink_swap_never_reowns_outside_the_root()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("tree");
    let outside = temp.path().join("outside");
    let dir = root.join("dir");
    fs::create_dir_all(&dir)?;
    fs::create_dir(&outside)?;
    for index in 0..200 {
        fs::write(outside.join(format!("victim-{index}")), "v")?;
        fs::write(dir.join(format!("victim-{index}")), "x")?;
    }
    let (uid, gid) = own_ids(&root);
    let Some(target) = foreign_group(gid, "the swap race is not exercised") else {
        return Ok(());
    };
    let stop = Arc::new(AtomicBool::new(false));
    let attacker = {
        let (dir, outside, stop) = (dir.clone(), outside.clone(), stop.clone());
        thread::spawn(move || {
            let parked = dir.with_file_name("dir.parked");
            while !stop.load(Ordering::Relaxed) {
                let _ = fs::rename(&dir, &parked);
                let _ = symlink(&outside, &dir);
                let _ = fs::remove_file(&dir);
                let _ = fs::rename(&parked, &dir);
            }
        })
    };

    let mut failures = Vec::new();
    let mut escaped = 0;
    for _ in 0..300 {
        // Alternate the group so every walk has work to do again.
        for group in [target, gid] {
            if let Err(error) = own_tree_no_follow(&root, uid, group) {
                failures.push(error);
            }
            escaped = (0..200)
                .filter(|index| {
                    fs::symlink_metadata(outside.join(format!("victim-{index}")))
                        .is_ok_and(|metadata| metadata.gid() != gid)
                })
                .count();
            if escaped > 0 {
                break;
            }
        }
        if escaped > 0 {
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    attacker
        .join()
        .unwrap_or_else(|_| panic!("attacker panicked"));

    assert_eq!(escaped, 0, "the walk re-owned files outside the tree");
    assert!(failures.is_empty(), "walks failed: {failures:?}");
    assert_eq!(own_ids(&outside).1, gid);
    Ok(())
}

/// A writer deleting and recreating files while the walk runs (as Agents do with
/// `.git/index.lock` and build output) never fails it: a vanished entry has nothing to re-own.
#[test]
fn entries_removed_concurrently_do_not_fail_the_walk() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let checkout = root.path().to_path_buf();
    for index in 0..2000 {
        fs::write(checkout.join(format!("file-{index}")), "x")?;
    }
    let (uid, gid) = own_ids(&checkout);
    let Some(target) = foreign_group(gid, "the concurrent re-own is not exercised") else {
        return Ok(());
    };
    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let (checkout, stop) = (checkout.clone(), stop.clone());
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                for index in 0..2000 {
                    let _ = fs::remove_file(checkout.join(format!("file-{index}")));
                }
                for index in 0..2000 {
                    let _ = fs::write(checkout.join(format!("file-{index}")), "x");
                }
            }
        })
    };

    let mut failures = Vec::new();
    for walk in 0..25 {
        let group = if walk % 2 == 0 { target } else { gid };
        if let Err(error) = own_tree_no_follow(&checkout, uid, group) {
            failures.push(error);
        }
    }
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap_or_else(|_| panic!("writer panicked"));

    assert!(failures.is_empty(), "walks failed: {failures:?}");
    Ok(())
}

/// Every descriptor the walk holds into the tree — the pinned entry and each listed directory —
/// is close-on-exec, so a process another thread spawns while a deep walk runs inherits none of
/// them. Checked from the hook, where the pinned entry and every listing above it are open.
#[test]
fn every_descriptor_the_walk_holds_is_close_on_exec() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::tempdir()?;
    // Canonical, since `/proc/self/fd` links name resolved paths.
    let root = temp.path().canonicalize()?;
    fs::create_dir_all(root.join("a/b/c"))?;
    fs::write(root.join("a/b/c/file"), "x")?;
    let (uid, gid) = own_ids(&root);

    let mut checked = 0;
    let mut inheritable = Vec::new();
    own_tree(&root, uid, gid, |_: &CStr| {
        for entry in fs::read_dir("/proc/self/fd").unwrap_or_else(|error| panic!("{error}")) {
            let Ok(entry) = entry else { continue };
            // A descriptor another thread closed meanwhile has nothing left to check.
            let Ok(target) = fs::read_link(entry.path()) else {
                continue;
            };
            if !target.starts_with(&root) {
                continue;
            }
            let fd = entry.file_name();
            let Ok(info) = fs::read_to_string(Path::new("/proc/self/fdinfo").join(&fd)) else {
                continue;
            };
            // `flags:` is octal; `O_CLOEXEC` is 02000000 there.
            let flags = info
                .lines()
                .find_map(|line| line.strip_prefix("flags:"))
                .and_then(|flags| u32::from_str_radix(flags.trim(), 8).ok())
                .unwrap_or_else(|| panic!("no flags for descriptor {fd:?}"));
            checked += 1;
            if flags & 0o2_000_000 == 0 {
                inheritable.push(target);
            }
        }
    })?;

    assert_eq!(inheritable, Vec::<std::path::PathBuf>::new());
    // root, a, a/b and a/b/c listed, plus each pinned entry: the check saw the walk's own.
    assert!(
        checked >= 8,
        "only {checked} descriptors into the tree were seen"
    );
    Ok(())
}
