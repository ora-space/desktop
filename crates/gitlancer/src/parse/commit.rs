use crate::domain::refs::CommitId;
use crate::error::ParseError;
use crate::git::commit::CommitResponse;

/// Parses commit metadata from a two-line payload that contains a commit ID followed by a summary.
pub fn parse_commit_response(stdout: &str) -> Result<CommitResponse, ParseError> {
    let mut lines = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let commit_id = lines.next().ok_or(ParseError::MissingLine)?;
    let summary = lines.next().ok_or(ParseError::MissingLine)?;

    Ok(CommitResponse {
        commit_id: CommitId::new(commit_id.to_string()),
        summary: summary.to_string(),
    })
}

/// Parses one commit identifier from the first non-empty line of stdout.
pub fn parse_commit_id(stdout: &str) -> Result<CommitId, ParseError> {
    let line = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or(ParseError::MissingLine)?;

    Ok(CommitId::new(line.to_string()))
}
