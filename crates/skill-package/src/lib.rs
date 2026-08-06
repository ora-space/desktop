//! Reads and validates skill packages from folder trees and supported archives.
//!
//! The crate materializes a security-checked snapshot of one logical source, scans it for
//! `SKILL.md` skill boundaries, and parses each manifest. It is transport- and persistence-
//! agnostic: callers own the destination directory, session lifecycle, and database writes.

pub mod archive;
pub mod error;
pub mod folder;
pub mod limits;
pub mod manifest;
pub mod path;
pub mod scan;
pub mod snapshot;

#[cfg(test)]
mod tests;

pub use archive::{ArchiveFormat, extract_archive};
pub use error::PrepareError;
pub use folder::copy_folder_to;
pub use limits::Limits;
pub use manifest::{
    Manifest, ManifestError, parse_manifest, render_manifest, render_minimal_manifest,
    rewrite_manifest, rewrite_manifest_body,
};
pub use path::{
    MAX_DEPTH, MAX_PATH_BYTES, MAX_SEGMENT_BYTES, MAX_SEGMENT_UTF16_UNITS, PathValidationError,
    RelativePath,
};
pub use scan::{SkillBoundary, scan_skill_boundaries};
pub use snapshot::{Snapshot, SnapshotFile, SnapshotWriter};
