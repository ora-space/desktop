//! Hard links name inodes, not paths. The walk leaves a file alone while another name links
//! it, which stops links that stay in place; a writer that removes its own name right after the
//! walk pinned the inode beats that check, so only a platform that refuses the link in the first
//! place (`fs.protected_hardlinks` on Linux) keeps foreign files out of the tree.

use super::{foreign_group, own_ids, own_tree, own_tree_no_follow};
use pretty_assertions::assert_eq;
use std::ffi::CStr;
use std::fs;
use std::os::unix::fs::MetadataExt;

/// A hard link that stays in place — planted before the walk, or swapped in for an entry before
/// or after the walk pinned it — does not make the walk re-own the outside inode it shares
/// (review 3's probe re-owned it). A file swapped after pinning is still re-owned as the inode
/// that was pinned.
#[test]
fn a_hard_link_that_stays_in_place_is_not_reowned() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let (uid, gid) = own_ids(root.path());
    let Some(target) = foreign_group(gid, "a hard-linked re-own is not observable") else {
        return Ok(());
    };
    let victim = |name: &str| outside.path().join(name);
    for name in ["planted", "swapped-early", "swapped-late"] {
        fs::write(victim(name), "v")?;
    }
    fs::hard_link(victim("planted"), root.path().join("planted"))?;
    for name in ["first", "second", "late"] {
        fs::write(root.path().join(name), "x")?;
    }
    let late = root.path().join("late");
    let mut pinned_late = None;
    let (first, second) = (root.path().join("first"), root.path().join("second"));

    let mut swapped_early = false;
    let result = own_tree(root.path(), uid, target, |name: &CStr| {
        let swapped = if name == c"late" {
            // Already pinned: the name now leads elsewhere, the descriptor does not.
            pinned_late = Some(fs::File::open(&late));
            fs::remove_file(&late).and_then(|()| fs::hard_link(victim("swapped-late"), &late))
        } else if !swapped_early && (name == c"first" || name == c"second") {
            // Not pinned yet: whichever of the two comes first swaps the other one.
            swapped_early = true;
            let other = if name == c"first" { &second } else { &first };
            fs::remove_file(other).and_then(|()| fs::hard_link(victim("swapped-early"), other))
        } else {
            Ok(())
        };
        swapped.unwrap_or_else(|error| panic!("{error}"));
    });

    // The root, the late file's pinned inode and whichever of first/second was not swapped.
    assert_eq!(result?, 3);
    let groups: Vec<_> = ["planted", "swapped-early", "swapped-late"]
        .into_iter()
        .map(|name| own_ids(&victim(name)).1)
        .collect();
    assert_eq!(groups, vec![gid; 3]);
    // The late file's original inode, now unlinked, is the one that was re-owned.
    let pinned_late = pinned_late.ok_or("the late file was never pinned")??;
    assert_eq!(pinned_late.metadata()?.gid(), target);
    Ok(())
}

/// A file legitimately hard-linked inside the tree keeps its owner unless it already has the
/// target one: the walk cannot tell its other name is not outside. A link count of one is
/// re-owned as usual, and an already-owned linked file is simply right.
#[test]
fn a_hard_linked_file_keeps_its_owner() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let (uid, gid) = own_ids(root.path());
    let Some(target) = foreign_group(gid, "a hard-linked re-own is not observable") else {
        return Ok(());
    };
    fs::write(root.path().join("one"), "x")?;
    fs::hard_link(root.path().join("one"), root.path().join("two"))?;
    fs::write(root.path().join("single"), "x")?;

    // The root and `single`; `one` and `two` are the same twice-linked inode.
    assert_eq!(own_tree_no_follow(root.path(), uid, target)?, 2);
    let groups: Vec<_> = ["one", "two", "single"]
        .into_iter()
        .map(|name| own_ids(&root.path().join(name)).1)
        .collect();
    assert_eq!(groups, vec![gid, gid, target]);
    // Once a name is gone the inode is the tree's alone and is re-owned.
    fs::remove_file(root.path().join("two"))?;
    assert_eq!(own_tree_no_follow(root.path(), uid, target)?, 1);
    assert_eq!(own_ids(&root.path().join("one")).1, target);
    Ok(())
}

/// The stated limit of the link-count skip: a writer that removes its own link right after the
/// walk pinned the inode leaves only the outside name, so the walk reads a link count of one and
/// re-owns the outside inode. Nothing in the walk can tell the two apart; only refusing the link
/// itself (`fs.protected_hardlinks`) keeps foreign files out (review 4's probe R4A).
#[test]
fn a_hard_link_removed_after_pinning_is_reowned() -> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let outside = tempfile::tempdir()?;
    let (uid, gid) = own_ids(root.path());
    let Some(target) = foreign_group(gid, "the raced re-own is not observable") else {
        return Ok(());
    };
    let victim = outside.path().join("victim");
    fs::write(&victim, "v")?;
    let planted = root.path().join("x");
    fs::hard_link(&victim, &planted)?;

    let result = own_tree(root.path(), uid, target, |name: &CStr| {
        if name == c"x" {
            fs::remove_file(&planted).unwrap_or_else(|error| panic!("{error}"));
        }
    });

    // The root and the outside inode the planted name pinned.
    assert_eq!(result?, 2);
    assert_eq!(own_ids(&victim).1, target);
    Ok(())
}
