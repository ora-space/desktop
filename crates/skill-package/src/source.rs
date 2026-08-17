use crate::limits::Limits;
use ora_utils::archive::{ArchiveError, ArchiveFormat, ExtractedTree, copy_tree, extract_archive};
use std::path::Path;

/// One logical skill source selected by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillSource<'a> {
    /// A local folder tree that is copied without following links.
    Folder { path: &'a Path },
    /// One supported archive whose format was derived from its file name.
    Archive {
        path: &'a Path,
        format: ArchiveFormat,
    },
}

/// Materializes one skill source into a validated snapshot under `destination`.
///
/// The snapshot never touches the formal skill directory; it only backs preview and per-skill
/// staging during commit. Every archive and path safety rule is enforced by
/// `ora-utils::archive` and rejects the whole source.
pub fn materialize_source(
    source: SkillSource<'_>,
    destination: &Path,
    limits: &Limits,
) -> Result<ExtractedTree, ArchiveError> {
    match source {
        SkillSource::Folder { path } => copy_tree(path, destination, &limits.extract),
        SkillSource::Archive { path, format } => {
            extract_archive(format, path, destination, &limits.extract)
        }
    }
}
