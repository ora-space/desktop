//! Safe materialization of untrusted archives and folder trees into a destination directory.
//!
//! Every entry path passes [`StrictRelativePath`](crate::path::StrictRelativePath) validation
//! before anything is written, so zip-slip, traversal, and platform-unsafe names never reach the
//! filesystem. Encrypted archives, symlinks, and special entries are rejected, and cumulative
//! entry-count and byte budgets abort the whole tree instead of a single entry.

mod copy;
mod error;
mod extract;
mod extracted;
mod format;
mod limits;
mod tar_entries;
mod tree_writer;
mod zip_entries;

#[cfg(test)]
mod tests;

pub use copy::copy_tree;
pub use error::ArchiveError;
pub use extract::extract_archive;
pub use extracted::{ExtractedFile, ExtractedTree};
pub use format::ArchiveFormat;
pub use limits::ExtractLimits;
