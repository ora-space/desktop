//! Mirrors a tree of regular files into a fresh view another identity can read.

use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// Mode of every directory the view creates: traversable and listable by anyone, writable only by
/// the creator, regardless of the caller's umask.
const VIEW_DIRECTORY_MODE: u32 = 0o755;

/// Builds `destination` as a view of the tree at `source`: the same directories, recreated with
/// mode `0o755`, holding hard links to the same regular files.
///
/// The source may sit behind owner-private directories (a umask of `077` leaves them `0700`), so
/// another identity cannot traverse it. Hard links give that identity the same inodes through new,
/// traversable directories without duplicating bytes, and keep each file's own mode and owner, so
/// the view grants nothing the file itself does not. When the destination lies on another
/// filesystem (`EXDEV`), a file is copied instead with a fixed `0o644` or `0o755` mode that
/// carries only the source's executability — never setuid, setgid, or sticky bits.
///
/// Entries are inspected with `symlink_metadata`: a symbolic link or special file anywhere in the
/// source fails the whole call instead of being followed or reproduced, so a link planted in the
/// source cannot extend the view to anything outside it. `destination` must not exist; on failure
/// whatever was already created stays for the caller to remove. The walk uses an explicit stack, so
/// deep trees cannot overflow the call stack.
pub fn link_tree_no_follow(source: &Path, destination: &Path) -> io::Result<()> {
    let root = fs::symlink_metadata(source)?;
    if !root.file_type().is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a directory", source.display()),
        ));
    }
    create_view_directory(destination)?;
    let mut pending: Vec<(PathBuf, PathBuf)> = vec![(source.to_path_buf(), destination.into())];
    while let Some((from, to)) = pending.pop() {
        for entry in fs::read_dir(&from)? {
            let entry = entry?;
            let source_path = entry.path();
            let destination_path = to.join(entry.file_name());
            let metadata = fs::symlink_metadata(&source_path)?;
            let file_type = metadata.file_type();
            if file_type.is_dir() {
                create_view_directory(&destination_path)?;
                pending.push((source_path, destination_path));
            } else if file_type.is_file() {
                match fs::hard_link(&source_path, &destination_path) {
                    Ok(()) => {}
                    Err(error) if error.raw_os_error() == Some(libc::EXDEV) => {
                        copy_regular_file(&source_path, &destination_path, metadata.mode())?;
                    }
                    Err(error) => return Err(error),
                }
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "{} is a link or special file and cannot join a view",
                        source_path.display()
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Creates one view directory and then sets its mode explicitly, because the creation mode is
/// still filtered by the caller's umask.
fn create_view_directory(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .mode(VIEW_DIRECTORY_MODE)
        .create(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(VIEW_DIRECTORY_MODE))
}

/// Copies one regular file for the cross-filesystem fallback, refusing a link swapped in at
/// `source` after it was inspected and keeping only the executability of `source_mode`.
fn copy_regular_file(source: &Path, destination: &Path, source_mode: u32) -> io::Result<()> {
    let mode = if source_mode & 0o111 == 0 {
        0o644
    } else {
        0o755
    };
    let mut reader = fs::OpenOptions::new()
        .read(/*read*/ true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(source)?;
    if !reader.metadata()?.file_type().is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is no longer a regular file", source.display()),
        ));
    }
    let mut writer = fs::OpenOptions::new()
        .write(/*write*/ true)
        .create_new(/*create_new*/ true)
        .mode(mode)
        .open(destination)?;
    io::copy(&mut reader, &mut writer)?;
    writer.sync_all()?;
    fs::set_permissions(destination, fs::Permissions::from_mode(mode))
}

#[cfg(test)]
mod tests {
    use super::{copy_regular_file, link_tree_no_follow};
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::fs;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::path::Path;

    /// Describes every entry under `root` as (kind, mode, inode) by relative path, for whole-tree
    /// comparison. Directory inodes are left out: a view's directories are new by construction,
    /// so only file inodes carry the "same inode" claim.
    fn describe(root: &Path) -> BTreeMap<String, (&'static str, u32, Option<u64>)> {
        let mut entries = BTreeMap::new();
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            for entry in fs::read_dir(&path).expect("read dir") {
                let entry = entry.expect("entry");
                let metadata = fs::symlink_metadata(entry.path()).expect("metadata");
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .expect("relative")
                    .to_string_lossy()
                    .into_owned();
                let described = if metadata.is_dir() {
                    pending.push(entry.path());
                    ("dir", metadata.mode() & 0o7777, None)
                } else {
                    ("file", metadata.mode() & 0o7777, Some(metadata.ino()))
                };
                entries.insert(relative, described);
            }
        }
        entries
    }

    /// Owner-private source directories become traversable view directories, and every file in
    /// the view is the very same inode with its own mode.
    #[test]
    fn links_files_into_traversable_directories() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("lib").join("nested"))?;
        fs::write(source.join("main.js"), "main")?;
        let tool = source.join("lib").join("nested").join("tool");
        fs::write(&tool, "tool")?;
        fs::set_permissions(source.join("main.js"), fs::Permissions::from_mode(0o644))?;
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755))?;
        for directory in [
            source.join("lib").join("nested"),
            source.join("lib"),
            source.clone(),
        ] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        }
        let destination = temp.path().join("view");

        link_tree_no_follow(&source, &destination)?;

        let inode = |path: &Path| fs::metadata(path).map(|metadata| Some(metadata.ino()));
        assert_eq!(
            (
                fs::metadata(&destination)?.mode() & 0o7777,
                describe(&destination)
            ),
            (
                0o755,
                BTreeMap::from([
                    ("lib".to_string(), ("dir", 0o755, None)),
                    ("lib/nested".to_string(), ("dir", 0o755, None)),
                    (
                        "lib/nested/tool".to_string(),
                        ("file", 0o755, inode(&tool)?)
                    ),
                    (
                        "main.js".to_string(),
                        ("file", 0o644, inode(&source.join("main.js"))?)
                    ),
                ])
            )
        );
        Ok(())
    }

    /// A symbolic link anywhere in the source fails the view instead of being followed.
    #[test]
    fn rejects_symbolic_links() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir_all(source.join("lib"))?;
        fs::write(temp.path().join("secret"), "secret")?;
        symlink(temp.path().join("secret"), source.join("lib/escape"))?;

        let error = link_tree_no_follow(&source, &temp.path().join("view"))
            .expect_err("a link must not join the view");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        Ok(())
    }

    /// A FIFO (or any other special file) fails the view instead of being reproduced.
    #[test]
    fn rejects_special_files() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir_all(&source)?;
        let fifo = CString::new(source.join("pipe").as_os_str().as_bytes())?;
        // SAFETY: mkfifo reads a valid NUL-terminated path and changes nothing else.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);

        let error = link_tree_no_follow(&source, &temp.path().join("view"))
            .expect_err("a FIFO must not join the view");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        Ok(())
    }

    /// An existing destination is never adopted: whatever it holds was not built from the source.
    #[test]
    fn refuses_an_existing_destination() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let source = temp.path().join("source");
        fs::create_dir_all(&source)?;
        let destination = temp.path().join("view");
        fs::create_dir_all(&destination)?;

        let error = link_tree_no_follow(&source, &destination)
            .expect_err("an existing destination must be refused");

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        Ok(())
    }

    /// The cross-filesystem copy keeps only executability, never special bits or the source mode.
    #[test]
    fn copies_keep_only_executability() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let plain = temp.path().join("plain");
        let tool = temp.path().join("tool");
        fs::write(&plain, "plain")?;
        fs::write(&tool, "tool")?;

        copy_regular_file(&plain, &temp.path().join("plain-copy"), 0o600)?;
        copy_regular_file(&tool, &temp.path().join("tool-copy"), 0o4700)?;

        let mode = |name: &str| fs::metadata(temp.path().join(name)).map(|m| m.mode() & 0o7777);
        assert_eq!(
            (
                mode("plain-copy")?,
                mode("tool-copy")?,
                fs::read_to_string(temp.path().join("tool-copy"))?,
            ),
            (0o644, 0o755, "tool".to_string()),
        );
        Ok(())
    }
}
