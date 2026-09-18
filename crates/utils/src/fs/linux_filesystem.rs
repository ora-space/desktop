use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;

/// Identifies common persistent local Linux filesystems without claiming device durability.
///
/// Callers choose their supported set. Unknown includes network, memory and stacked filesystems;
/// a magic number cannot certify mount options, backing devices or crash behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxFilesystem {
    Ext,
    Xfs,
    Btrfs,
    Other,
}

impl LinuxFilesystem {
    /// Probes an open inode so classification does not resolve a second pathname.
    pub fn for_file(file: &File) -> io::Result<Self> {
        let mut filesystem = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: file is live and fstatfs initializes the output on success.
        if unsafe { libc::fstatfs(file.as_raw_fd(), filesystem.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: the successful call initialized this structure.
        let filesystem = unsafe { filesystem.assume_init() };
        Ok(match filesystem.f_type {
            libc::EXT4_SUPER_MAGIC => Self::Ext,
            libc::XFS_SUPER_MAGIC => Self::Xfs,
            libc::BTRFS_SUPER_MAGIC => Self::Btrfs,
            _ => Self::Other,
        })
    }
}
