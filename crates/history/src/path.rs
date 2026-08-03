use crate::error::HistoryError;
use std::path::{Path, PathBuf};

/// Splits the session tree two levels deep, the way Git shards its object store.
const SHARD_WIDTH: usize = 2;

/// Derives the history file path that belongs to one Ora session identifier.
///
/// Session identifiers are random UUID v4 values, so the leading hex characters
/// are uniformly distributed and split the tree into 65,536 directories. The path
/// is derived rather than stored because a second recorded location would be a
/// second thing that can disagree about where a session's history lives.
pub fn history_path(root: &Path, session_id: &str) -> Result<PathBuf, HistoryError> {
    let identifier = validated_identifier(session_id)?;
    let (first, rest) = identifier.split_at(SHARD_WIDTH.min(identifier.len()));
    let second = &rest[..SHARD_WIDTH.min(rest.len())];
    Ok(root
        .join(first)
        .join(second)
        .join(format!("{identifier}.jsonl")))
}

/// Rejects any identifier that would not be a single, self-contained file name.
///
/// The identifier becomes a path component, so this is the boundary where a value
/// that never came from Ora's own generator has to be refused rather than
/// resolved. Every identifier Ora mints is a UUID and passes unchanged.
fn validated_identifier(session_id: &str) -> Result<&str, HistoryError> {
    let usable = !session_id.is_empty()
        && session_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-');
    if usable {
        Ok(session_id)
    } else {
        Err(HistoryError::InvalidSessionId {
            session_id: session_id.to_string(),
        })
    }
}
