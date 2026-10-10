//! Hands a directory tree to one owner without following links.

use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, lchown};
use std::path::Path;

/// Gives every entry under `root` (and `root` itself) to `uid`:`gid`, returning how many entries
/// changed owner.
///
/// Entries are inspected with `symlink_metadata` and changed with `lchown`, so a symbolic link is
/// re-owned as a link and never followed: a link planted inside the tree cannot make a privileged
/// caller change the owner of anything outside it, and a linked directory is never descended
/// into. Entries already owned by `uid`:`gid` are left untouched, so a tree that is already right
/// costs only a walk. The walk is depth-first with an explicit stack, so deep trees cannot
/// overflow the call stack. Permission bits are not changed: the new owner gets whatever access
/// the owner bits already grant.
pub fn own_tree_no_follow(root: &Path, uid: u32, gid: u32) -> io::Result<usize> {
    let mut changed = 0;
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.uid() != uid || metadata.gid() != gid {
            lchown(&path, Some(uid), Some(gid))?;
            changed += 1;
        }
        if metadata.file_type().is_dir() {
            for entry in fs::read_dir(&path)? {
                pending.push(entry?.path());
            }
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::own_tree_no_follow;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

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
        let metadata = fs::metadata(root.path())?;

        let result = own_tree_no_follow(root.path(), metadata.uid(), metadata.gid());
        fs::set_permissions(&sealed, fs::Permissions::from_mode(0o700))?;
        assert_eq!(result?, 0);
        Ok(())
    }

    /// A tree the caller may not re-own reports the error instead of silently skipping it.
    #[test]
    fn reports_entries_it_cannot_reown() -> Result<(), Box<dyn std::error::Error>> {
        // Root may re-own anything, so the refusal is only observable unprivileged.
        if nix_is_root() {
            return Ok(());
        }
        let root = tempfile::tempdir()?;
        fs::write(root.path().join("file"), "x")?;
        assert!(own_tree_no_follow(root.path(), 0, 0).is_err());
        Ok(())
    }

    /// Reads the effective uid through the filesystem the test already created, avoiding a libc
    /// dependency in this crate's tests.
    fn nix_is_root() -> bool {
        tempfile::tempdir()
            .ok()
            .and_then(|dir| fs::metadata(dir.path()).ok())
            .is_some_and(|metadata| metadata.uid() == 0)
    }
}
