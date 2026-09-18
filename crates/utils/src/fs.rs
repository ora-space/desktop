//! Filesystem helpers for safe naming and file ownership coordination.
//!
//! [`sanitize_file_name`] turns arbitrary text (download suggestions, URL segments, user input)
//! into one portable basename, and [`next_available_file_name`] picks a collision-free variant of
//! that basename inside a directory. Both treat everything after the last `.` as the extension;
//! see the module README for why multi-part extensions such as `.tar.gz` are not special-cased.
//!
//! On Linux, `LinuxFileLock` supplies transferable advisory ownership of an already opened file.
//! Trusted path resolution and persistent filesystem layout remain the caller's responsibility.
//! `LinuxFilesystem` classifies an open inode's filesystem; callers choose their own supported
//! set and must separately verify mount/device durability and failure behavior.

mod file_name;
#[cfg(target_os = "linux")]
mod linux_file_lock;
#[cfg(target_os = "linux")]
mod linux_filesystem;
mod unique_path;

#[cfg(target_os = "linux")]
pub use linux_file_lock::LinuxFileLock;
#[cfg(target_os = "linux")]
pub use linux_filesystem::LinuxFilesystem;

pub use file_name::sanitize_file_name;
pub use unique_path::next_available_file_name;
