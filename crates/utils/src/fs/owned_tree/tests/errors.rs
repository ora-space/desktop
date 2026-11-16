//! Every failure that is not a vanished entry is reported, at each step an entry goes through.
//!
//! Root bypasses the permission checks these tests rely on, so they only assert unprivileged.
//! The re-own itself cannot be made to fail for an entry below the root without privilege (an
//! unprivileged caller may always hand its own inodes to one of its groups, and cannot bring
//! anyone else's inode into the tree); the root and every entry share one re-own step, so the
//! root's refusal covers it.

use super::{is_root, os_error, own_tree, own_tree_no_follow};
use pretty_assertions::assert_eq;
use std::ffi::CStr;
use std::fs;
use std::os::unix::fs::PermissionsExt;

/// A tree the caller may not re-own reports the error instead of silently skipping it.
#[test]
fn reports_entries_it_cannot_reown() -> Result<(), Box<dyn std::error::Error>> {
    if is_root() {
        return Ok(());
    }
    let root = tempfile::tempdir()?;
    fs::write(root.path().join("file"), "x")?;
    let refused = own_tree_no_follow(root.path(), 0, 0).map_err(os_error);
    // `EINVAL` where id 0 is not mapped at all, as in a user namespace without a mapping.
    assert!(
        matches!(refused, Err(Some(libc::EPERM | libc::EINVAL))),
        "{refused:?}"
    );
    Ok(())
}

/// A directory below the root the caller may not list fails the walk.
#[test]
fn reports_a_directory_it_cannot_list() -> Result<(), Box<dyn std::error::Error>> {
    if is_root() {
        return Ok(());
    }
    let root = tempfile::tempdir()?;
    let (uid, gid) = super::own_ids(root.path());
    let locked = root.path().join("a/locked");
    fs::create_dir_all(&locked)?;
    fs::write(locked.join("file"), "x")?;
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000))?;

    let result = own_tree_no_follow(root.path(), uid, gid);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700))?;
    assert_eq!(result.map_err(os_error), Err(Some(libc::EACCES)));
    Ok(())
}

/// An entry the caller may no longer reach once its directory was listed fails the walk: here
/// the directory loses its search permission after its first entry was pinned, so pinning the
/// second one is refused.
#[test]
fn reports_an_entry_it_cannot_pin() -> Result<(), Box<dyn std::error::Error>> {
    if is_root() {
        return Ok(());
    }
    let root = tempfile::tempdir()?;
    let (uid, gid) = super::own_ids(root.path());
    let directory = root.path().join("dir");
    fs::create_dir(&directory)?;
    for name in ["x", "y"] {
        fs::write(directory.join(name), "x")?;
    }

    let mut locked = false;
    let result = own_tree(root.path(), uid, gid, |name: &CStr| {
        if !locked && (name == c"x" || name == c"y") {
            locked = true;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o000))
                .unwrap_or_else(|error| panic!("{error}"));
        }
    });
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    assert_eq!(result.map_err(os_error), Err(Some(libc::EACCES)));
    Ok(())
}
