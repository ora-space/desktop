//! Filesystem helpers for files whose names originate from untrusted sources.
//!
//! [`sanitize_file_name`] turns arbitrary text (download suggestions, URL segments, user input)
//! into one portable basename, and [`next_available_file_name`] picks a collision-free variant of
//! that basename inside a directory. Both treat everything after the last `.` as the extension;
//! see the module README for why multi-part extensions such as `.tar.gz` are not special-cased.

mod file_name;
mod unique_path;

pub use file_name::sanitize_file_name;
pub use unique_path::next_available_file_name;
