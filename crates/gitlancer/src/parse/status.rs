use crate::error::ParseError;
use crate::git::status::StatusEntry;

/// Parses porcelain-v2 `-z` output into raw status entries while preserving Git's machine-readable records.
pub fn parse_status_v2(stdout: &str) -> Result<Vec<StatusEntry>, ParseError> {
    let entries = stdout
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .filter(|entry| !entry.starts_with('#'))
        .map(|entry| StatusEntry {
            raw: entry.to_string(),
        })
        .collect::<Vec<_>>();

    Ok(entries)
}
