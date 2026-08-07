use crate::domain::refs::CommitId;
use crate::error::ParseError;
use crate::git::history::{
    CommitDetails, CommitFile, CommitSummary, ReferenceKind, RepositoryReference,
};

const COMMIT_RECORD_SEPARATOR: char = '\u{1e}';

/// Parses the NUL-and-record-separated stream emitted by \`git log --format\`.
pub fn parse_commit_history(stdout: &str) -> Result<Vec<CommitSummary>, ParseError> {
    stdout
        .split(COMMIT_RECORD_SEPARATOR)
        .filter(|record| !record.trim().is_empty())
        .map(parse_commit_record)
        .collect()
}

/// Parses one commit detail stream into its metadata and name-status file list.
pub fn parse_commit_details(stdout: &str) -> Result<CommitDetails, ParseError> {
    let (metadata, file_output) = stdout
        .split_once(COMMIT_RECORD_SEPARATOR)
        .ok_or(ParseError::InvalidCommitDetails)?;
    let summary = parse_commit_record(metadata)?;
    let files = file_output
        .lines()
        .filter_map(parse_commit_file)
        .collect::<Vec<_>>();

    Ok(CommitDetails { summary, files })
}

/// Parses full ref names and target object ids from \`git for-each-ref\` output.
pub fn parse_references(stdout: &str) -> Result<Vec<RepositoryReference>, ParseError> {
    stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let (full_name, object_id) = line
                .split_once('\t')
                .ok_or(ParseError::InvalidCommitHistory)?;
            let (name, kind) = reference_name_and_kind(full_name)?;
            if object_id.is_empty() {
                return Err(ParseError::InvalidCommitHistory);
            }

            Ok(RepositoryReference {
                name: name.to_string(),
                commit_id: CommitId::new(object_id),
                kind,
            })
        })
        .collect()
}

/// Parses one machine-readable commit record while keeping subject text intact.
fn parse_commit_record(record: &str) -> Result<CommitSummary, ParseError> {
    let fields = record
        .trim_matches(|character| character == '\r' || character == '\n')
        .split('\0')
        .collect::<Vec<_>>();
    if fields.len() != 6
        || fields[0].is_empty()
        || fields[2].is_empty()
        || fields[3].is_empty()
        || fields[4].is_empty()
    {
        return Err(ParseError::InvalidCommitHistory);
    }

    let parents = fields[1]
        .split_whitespace()
        .map(CommitId::new)
        .collect::<Vec<_>>();

    Ok(CommitSummary {
        id: CommitId::new(fields[0]),
        parents,
        subject: fields[5].to_string(),
        author_name: fields[2].to_string(),
        author_email: fields[3].to_string(),
        authored_at: fields[4].to_string(),
    })
}

/// Parses one Git name-status line, selecting the destination path for rename records.
fn parse_commit_file(line: &str) -> Option<CommitFile> {
    let mut fields = line.split('\t');
    let status = fields.next()?.trim();
    let path = line.rsplit('\t').next()?.trim();
    if status.is_empty() || path.is_empty() || !status.chars().next()?.is_ascii_alphabetic() {
        return None;
    }

    Some(CommitFile {
        status: status.to_string(),
        path: path.to_string(),
    })
}

/// Maps a full ref name to the short name and graph label category exposed upstream.
fn reference_name_and_kind(full_name: &str) -> Result<(&str, ReferenceKind), ParseError> {
    if let Some(name) = full_name.strip_prefix("refs/heads/") {
        return Ok((name, ReferenceKind::Local));
    }
    if let Some(name) = full_name.strip_prefix("refs/remotes/") {
        return Ok((name, ReferenceKind::Remote));
    }
    if let Some(name) = full_name.strip_prefix("refs/tags/") {
        return Ok((name, ReferenceKind::Tag));
    }

    Err(ParseError::InvalidCommitHistory)
}

#[cfg(test)]
mod tests {
    use super::{parse_commit_details, parse_commit_history, parse_references};
    use crate::git::history::ReferenceKind;
    use pretty_assertions::assert_eq;

    /// Verifies commit records preserve topology and author metadata needed by a graph renderer.
    #[test]
    fn parses_commit_history_records() {
        let output = concat!(
            "abc123\0parent1 parent2\0Ora Tests\0ora@example.com\02026-08-04T10:00:00+08:00\0merge feature\u{1e}\n",
            "parent1\0\0Ora Tests\0ora@example.com\02026-08-03T10:00:00+08:00\0initial\u{1e}\n",
        );

        let commits = parse_commit_history(output).expect("parse history");

        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].parents.len(), 2);
        assert_eq!(commits[0].subject, "merge feature");
        assert_eq!(commits[1].parents, Vec::new());
    }

    /// Verifies commit details select the destination path for renames and ignore formatting blanks.
    #[test]
    fn parses_commit_details_files() {
        let output = concat!(
            "abc123\0parent\0Ora Tests\0ora@example.com\02026-08-04T10:00:00+08:00\0rename file\u{1e}\n",
            "\nR100\told.md\tnew.md\nM\tREADME.md\n",
        );

        let details = parse_commit_details(output).expect("parse details");

        assert_eq!(details.files.len(), 2);
        assert_eq!(details.files[0].path, "new.md");
        assert_eq!(details.files[0].status, "R100");
    }

    /// Verifies full ref names become stable short labels and typed categories.
    #[test]
    fn parses_reference_categories() {
        let references = parse_references(
            "refs/heads/main\tabc\nrefs/remotes/origin/main\tdef\nrefs/tags/v1\tghi\n",
        )
        .expect("parse references");

        assert_eq!(references[0].kind, ReferenceKind::Local);
        assert_eq!(references[1].name, "origin/main");
        assert_eq!(references[2].kind, ReferenceKind::Tag);
    }
}
