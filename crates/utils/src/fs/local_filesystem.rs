//! Kernel-owned local filesystems suitable for durable OFD ownership evidence.
use std::{fs::File, io, os::fd::AsRawFd, path::Path};

/// Refuses network or unknown filesystems instead of inferring local lock semantics from paths.
pub fn require_local_filesystem(path: &Path) -> io::Result<()> {
    let directory = File::open(path)?;
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: fstatfs fills the allocated structure and the descriptor remains open for the call.
    if unsafe { libc::fstatfs(directory.as_raw_fd(), info.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the successful call initialized the structure.
    let filesystem = unsafe { info.assume_init() }.f_type;
    // ext2/3/4, XFS, Btrfs, tmpfs, overlay, ZFS, F2FS, ramfs. Unknown includes network/FUSE.
    if matches!(
        filesystem,
        0xef53
            | 0x58465342
            | 0x9123683e
            | 0x01021994
            | 0x794c7630
            | 0x2fc12fc1
            | 0xf2f52010
            | 0x858458f6
    ) {
        Ok(())
    } else {
        Err(io::Error::other(
            "stop evidence requires a supported local filesystem",
        ))
    }
}

#[cfg(test)]
mod tests {
    /// The kernel reports filesystem identity independently of path names or mount aliases.
    #[test]
    fn local_temporary_storage_is_accepted_but_proc_is_not() -> std::io::Result<()> {
        let directory = tempfile::tempdir()?;
        super::require_local_filesystem(directory.path())?;
        assert!(super::require_local_filesystem(std::path::Path::new("/proc")).is_err());
        Ok(())
    }
}
