use crate::error::ParseError;
use crate::git::checkpoint::{ChangeStatus, ChangedPath};
use std::collections::BTreeMap;

/// One `--numstat` record: path, additions, deletions. Binary files use `None` counts.
type NumstatEntry = (String, Option<u64>, Option<u64>);

/// Parses `git diff-tree -r -z --name-status` output into per-path change records.
///
/// Rename and copy records consume two NUL-terminated paths (old then new). Every other status
/// consumes a single path. Unknown status letters are preserved as `ChangeStatus::Other`.
pub fn parse_name_status_z(stdout: &str) -> Result<Vec<ChangedPath>, ParseError> {
    let mut tokens = stdout.split('\0').filter(|token| !token.is_empty());
    let mut entries = Vec::new();
    while let Some(status) = tokens.next() {
        let (path, change_status) = if is_two_path_status(status) {
            let from = tokens
                .next()
                .ok_or(ParseError::InvalidCheckpoint)?
                .to_string();
            let path = tokens
                .next()
                .ok_or(ParseError::InvalidCheckpoint)?
                .to_string();
            let change_status = if status.starts_with('R') {
                ChangeStatus::Renamed { from }
            } else {
                ChangeStatus::Other(status.to_string())
            };
            (path, change_status)
        } else {
            let path = tokens
                .next()
                .ok_or(ParseError::InvalidCheckpoint)?
                .to_string();
            (path, status_from_code(status))
        };
        entries.push(ChangedPath {
            path,
            status: change_status,
            additions: None,
            deletions: None,
        });
    }
    Ok(entries)
}

/// Parses `git diff-tree -r -z --numstat` output into `(path, additions, deletions)` records.
///
/// Git emits `<added>\t<deleted>\t<path>\0`. Binary files report `-` for both counts and become
/// `None`. Rename and copy records use an empty path field followed by `from\0to\0`; the new path
/// is the join key used by [`combine_name_status_and_numstat`].
pub fn parse_numstat_z(stdout: &str) -> Result<Vec<NumstatEntry>, ParseError> {
    let tokens: Vec<&str> = stdout
        .split('\0')
        .filter(|token| !token.is_empty())
        .collect();
    let mut entries = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let parts: Vec<&str> = tokens[index].split('\t').collect();
        if parts.len() < 2 {
            return Err(ParseError::InvalidCheckpoint);
        }
        let additions = parse_numstat_count(parts[0])?;
        let deletions = parse_numstat_count(parts[1])?;
        let embedded_path = parts.get(2).copied().unwrap_or("");
        if !embedded_path.is_empty() {
            if parts.len() > 3 {
                return Err(ParseError::InvalidCheckpoint);
            }
            entries.push((embedded_path.to_string(), additions, deletions));
            index += 1;
            continue;
        }
        // Rename/copy: empty path field, then old path and new path as the next two tokens.
        // A two-field `added\tdeleted` token uses the next token as a single path.
        let first_path = tokens
            .get(index + 1)
            .copied()
            .ok_or(ParseError::InvalidCheckpoint)?;
        if index + 2 < tokens.len() && !tokens[index + 2].contains('\t') {
            entries.push((tokens[index + 2].to_string(), additions, deletions));
            index += 3;
        } else {
            entries.push((first_path.to_string(), additions, deletions));
            index += 2;
        }
    }
    Ok(entries)
}

/// Joins `--name-status` records with `--numstat` counts keyed by the new path.
pub fn combine_name_status_and_numstat(
    mut entries: Vec<ChangedPath>,
    stats: Vec<NumstatEntry>,
) -> Vec<ChangedPath> {
    let counts: BTreeMap<String, (Option<u64>, Option<u64>)> = stats
        .into_iter()
        .map(|(path, additions, deletions)| (path, (additions, deletions)))
        .collect();
    for entry in &mut entries {
        if let Some((additions, deletions)) = counts.get(&entry.path) {
            entry.additions = *additions;
            entry.deletions = *deletions;
        }
    }
    entries
}

/// Rename and copy status codes are followed by the old path and then the new path.
fn is_two_path_status(status: &str) -> bool {
    status.starts_with('R') || status.starts_with('C')
}

/// Maps a `--name-status` code onto the typed checkpoint status.
fn status_from_code(status: &str) -> ChangeStatus {
    match status {
        "A" => ChangeStatus::Added,
        "M" => ChangeStatus::Modified,
        "D" => ChangeStatus::Deleted,
        other => ChangeStatus::Other(other.to_string()),
    }
}

/// Parses one numstat integer, or `None` for Git's binary `-` placeholder.
fn parse_numstat_count(value: &str) -> Result<Option<u64>, ParseError> {
    if value == "-" {
        return Ok(None);
    }
    value
        .parse()
        .map(Some)
        .map_err(|_| ParseError::InvalidCheckpoint)
}

#[cfg(test)]
mod tests {
    use super::{combine_name_status_and_numstat, parse_name_status_z, parse_numstat_z};
    use crate::git::checkpoint::{ChangeStatus, ChangedPath};
    use pretty_assertions::assert_eq;

    /// Verifies NUL-delimited name-status records including a rename `R100` pair.
    #[test]
    fn parses_name_status_z_including_a_rename() {
        let stdout = "A\0added.txt\0M\0modified.txt\0D\0deleted.txt\0R100\0old.txt\0new.txt\0";
        assert_eq!(
            parse_name_status_z(stdout).unwrap(),
            vec![
                ChangedPath {
                    path: "added.txt".to_string(),
                    status: ChangeStatus::Added,
                    additions: None,
                    deletions: None,
                },
                ChangedPath {
                    path: "modified.txt".to_string(),
                    status: ChangeStatus::Modified,
                    additions: None,
                    deletions: None,
                },
                ChangedPath {
                    path: "deleted.txt".to_string(),
                    status: ChangeStatus::Deleted,
                    additions: None,
                    deletions: None,
                },
                ChangedPath {
                    path: "new.txt".to_string(),
                    status: ChangeStatus::Renamed {
                        from: "old.txt".to_string(),
                    },
                    additions: None,
                    deletions: None,
                },
            ]
        );
    }

    /// Verifies NUL-delimited numstat records, including a binary `-` line and a rename pair.
    #[test]
    fn parses_numstat_z_including_binary_and_rename() {
        // `git diff-tree --numstat -z` embeds the path after a second tab. Renames use an empty
        // path field followed by the old path and the new path as separate NUL tokens.
        let stdout =
            "1\t0\tadded.txt\02\t3\tmodified.txt\0-\t-\tbinary.bin\01\t1\t\0old.txt\0new.txt\0";
        assert_eq!(
            parse_numstat_z(stdout).unwrap(),
            vec![
                ("added.txt".to_string(), Some(1), Some(0)),
                ("modified.txt".to_string(), Some(2), Some(3)),
                ("binary.bin".to_string(), None, None),
                ("new.txt".to_string(), Some(1), Some(1)),
            ]
        );
    }

    /// Joins status and numstat records by the new path so callers see one row per file.
    #[test]
    fn combines_name_status_with_numstat_counts() {
        let entries =
            parse_name_status_z("A\0added.txt\0R100\0old.txt\0new.txt\0A\0binary.bin\0").unwrap();
        let stats =
            parse_numstat_z("1\t0\tadded.txt\01\t1\t\0old.txt\0new.txt\0-\t-\tbinary.bin\0")
                .unwrap();
        assert_eq!(
            combine_name_status_and_numstat(entries, stats),
            vec![
                ChangedPath {
                    path: "added.txt".to_string(),
                    status: ChangeStatus::Added,
                    additions: Some(1),
                    deletions: Some(0),
                },
                ChangedPath {
                    path: "new.txt".to_string(),
                    status: ChangeStatus::Renamed {
                        from: "old.txt".to_string(),
                    },
                    additions: Some(1),
                    deletions: Some(1),
                },
                ChangedPath {
                    path: "binary.bin".to_string(),
                    status: ChangeStatus::Added,
                    additions: None,
                    deletions: None,
                },
            ]
        );
    }
}
