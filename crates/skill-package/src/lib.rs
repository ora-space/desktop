//! Reads and validates skill packages from folder trees and supported archives.
//!
//! The crate materializes a security-checked snapshot of one logical source through
//! `ora-utils::archive`, scans it for `SKILL.md` skill boundaries, and parses each manifest. It
//! is transport- and persistence-agnostic: callers own the destination directory, session
//! lifecycle, and database writes.

pub mod limits;
pub mod manifest;
pub mod scan;
pub mod source;

#[cfg(test)]
mod tests;

pub use limits::Limits;
pub use manifest::{
    Manifest, ManifestError, parse_manifest, render_manifest, render_minimal_manifest,
    rewrite_manifest, rewrite_manifest_body,
};
pub use ora_utils::archive::{ArchiveError, ArchiveFormat, ExtractedFile, ExtractedTree};
pub use scan::{SKILL_MANIFEST_FILE_NAME, SkillBoundary, scan_skill_boundaries};
pub use source::{SkillSource, materialize_source};
