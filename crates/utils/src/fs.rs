//! Filesystem helpers for files whose names originate from untrusted sources and for files that
//! several writers might contend for.
//!
//! [`sanitize_file_name`] turns arbitrary text (download suggestions, URL segments, user input)
//! into one portable basename, and [`next_available_file_name`] picks a collision-free variant of
//! that basename inside a directory. Both treat everything after the last `.` as the extension;
//! see the module README for why multi-part extensions such as `.tar.gz` are not special-cased.
//!
//! [`refuse_final_link`] opens a hostile path without following a link at its last component,
//! [`ExclusiveFileLock`] serializes writers across processes through a sidecar lock file, and
//! [`classify_line_tail`] tells an appender whether a line-oriented file ends mid-line.
//! [`own_tree_no_follow`] hands a directory tree to one owner without following links, and
//! [`link_tree_no_follow`] mirrors a tree of regular files into a fresh, world-traversable view.

mod exclusive_lock;
mod file_name;
#[cfg(unix)]
mod linked_tree;
mod no_follow;
#[cfg(unix)]
mod owned_tree;
mod rename;
mod trailing_newline;
mod unique_path;

pub use exclusive_lock::{ExclusiveFileLock, ExclusiveLockError};
pub use file_name::sanitize_file_name;
#[cfg(unix)]
pub use linked_tree::link_tree_no_follow;
pub use no_follow::refuse_final_link;
#[cfg(unix)]
pub use owned_tree::own_tree_no_follow;
pub use rename::rename_with_retry;
pub use trailing_newline::{LineTail, classify_line_tail};
pub use unique_path::next_available_file_name;
