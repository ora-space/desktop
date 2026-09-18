use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
use std::path::{Component, Path};

/// Restricts the final inode so devices, sockets and FIFOs cannot masquerade as configuration.
#[derive(Clone, Copy)]
pub enum TrustedPathKind {
    File,
    /// Opens without truncation, including write-only kernel control files.
    WritableFile,
    Directory,
}

/// Opens an owner-private target under trusted ancestors, without changing existing permissions.
///
/// Regular files must have one link so a second pathname cannot accidentally share private state.
/// Ancestors follow `open_trusted_path` rules; the final target must belong to the selected owner
/// and deny all group/other access. This does not isolate mutually untrusted code with the same UID.
pub fn open_private_path(path: &Path, owner: u32, kind: TrustedPathKind) -> io::Result<File> {
    let file = open_trusted_path(path, owner, kind)?;
    let metadata = file.metadata()?;
    if metadata.uid() != owner
        || metadata.mode() & 0o077 != 0
        || (metadata.is_file() && metadata.nlink() != 1)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "target is not an owner-private inode",
        ));
    }
    Ok(file)
}

/// Opens an absolute path without following links, trusting only root and the selected owner.
///
/// Every ancestor and the final inode must reject group/other writes. Descriptor-relative walks
/// pin checked directories; the returned descriptor pins the checked target. Trusted owners may
/// still change their files, so this is not a sandbox against the selected owner or root.
pub fn open_trusted_path(path: &Path, owner: u32, kind: TrustedPathKind) -> io::Result<File> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected an absolute path without parent traversal",
        ));
    }
    let mut directory = File::open(Path::new(std::path::MAIN_SEPARATOR_STR))?;
    let parts: Vec<_> = path
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name),
            Component::RootDir
            | Component::CurDir
            | Component::ParentDir
            | Component::Prefix(_) => None,
        })
        .collect();
    for index in 0..=parts.len() {
        let metadata = directory.metadata()?;
        if (metadata.uid() != 0 && metadata.uid() != owner) || metadata.mode() & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "path is not exclusively controlled by trusted owners",
            ));
        }
        if index == parts.len() {
            let valid = match kind {
                TrustedPathKind::File | TrustedPathKind::WritableFile => metadata.is_file(),
                TrustedPathKind::Directory => metadata.is_dir(),
            };
            return if valid {
                Ok(directory)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "unexpected trusted path inode type",
                ))
            };
        }
        let name = CString::new(parts[index].as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains NUL"))?;
        let access = if index + 1 == parts.len() && matches!(kind, TrustedPathKind::WritableFile) {
            libc::O_WRONLY
        } else {
            libc::O_RDONLY
        };
        let mut flags = access | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK;
        if index + 1 < parts.len() || matches!(kind, TrustedPathKind::Directory) {
            flags |= libc::O_DIRECTORY;
        }
        // SAFETY: the directory descriptor and NUL-terminated component remain live for openat.
        let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: openat returned a fresh owned descriptor, transferred exactly once to File.
        directory = unsafe { File::from_raw_fd(fd) };
    }
    unreachable!("the final inode returns from the loop")
}
