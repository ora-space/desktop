use crate::workspace::{canonical_root, relative_string};
use crate::{WorkspaceFileSystem, WorkspaceFileSystemError};
use ora_process::{ManagedProcess, ProcessSpawner, ProcessSpec, ProcessStdio};
use ora_utils::path::CanonicalPathRoot;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};

pub(crate) const MAX_SEARCH_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_SEARCH_RESULTS: usize = 10_000;
pub(crate) const SEARCH_TIMEOUT: Duration = Duration::from_secs(15);

/// Chooses filename discovery or text-content matching.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchKind {
    Files,
    Content,
}

/// Describes one line-oriented text match.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchMatch {
    pub path: String,
    pub line: u64,
    pub column: u64,
    pub matched_text: String,
    pub preview: String,
}

/// Keeps filename and content results structurally distinct.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchResult {
    File { path: String },
    Match(SearchMatch),
}

/// Returns bounded search results and reports whether the result limit truncated them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
    pub truncated: bool,
}

impl<S> WorkspaceFileSystem<S>
where
    S: ProcessSpawner,
{
    /// Runs bundled ripgrep inside one canonical workspace and parses its bounded output.
    pub async fn search(
        &self,
        root: &Path,
        query: &str,
        kind: SearchKind,
    ) -> Result<SearchResults, WorkspaceFileSystemError> {
        let root = canonical_root(root)?;
        if kind == SearchKind::Content && query.trim().is_empty() {
            return Ok(SearchResults {
                results: Vec::new(),
                truncated: false,
            });
        }

        let arguments = search_arguments(query, kind);

        let spec = ProcessSpec::new(self.ripgrep_path.as_os_str())
            .args(arguments)
            .cwd(root.as_path())
            .stdin(ProcessStdio::Null);
        let mut process = self.process_spawner.spawn(spec).map_err(|source| {
            WorkspaceFileSystemError::SearchToolUnavailable {
                path: self.ripgrep_path.clone(),
                source,
            }
        })?;
        let stdout =
            process
                .take_stdout()
                .ok_or_else(|| WorkspaceFileSystemError::SearchFailed {
                    message: "ripgrep stdout pipe is unavailable".to_string(),
                })?;
        let stderr =
            process
                .take_stderr()
                .ok_or_else(|| WorkspaceFileSystemError::SearchFailed {
                    message: "ripgrep stderr pipe is unavailable".to_string(),
                })?;

        let completion = collect_process_output(&process, stdout, stderr);
        let (status, stdout, stderr) = match tokio::time::timeout(SEARCH_TIMEOUT, completion).await
        {
            Ok(result) => result?,
            Err(_) => {
                let _ = process.kill().await;
                return Err(WorkspaceFileSystemError::SearchTimedOut);
            }
        };
        if stdout.len() > MAX_SEARCH_OUTPUT_BYTES || stderr.len() > MAX_SEARCH_OUTPUT_BYTES {
            return Err(WorkspaceFileSystemError::SearchOutputTooLarge {
                limit_bytes: MAX_SEARCH_OUTPUT_BYTES,
            });
        }
        if !status.success() && status.code() != Some(1) {
            return Err(WorkspaceFileSystemError::SearchFailed {
                message: String::from_utf8_lossy(&stderr).trim().to_string(),
            });
        }

        match kind {
            SearchKind::Files => parse_files(&root, query, &stdout),
            SearchKind::Content => parse_matches(&root, &stdout),
        }
    }
}

/// Builds the ripgrep arguments while keeping user input in the value position.
fn search_arguments(query: &str, kind: SearchKind) -> Vec<String> {
    match kind {
        SearchKind::Files => vec![
            "--files".to_string(),
            "--hidden".to_string(),
            "--glob".to_string(),
            "!.git".to_string(),
            ".".to_string(),
        ],
        SearchKind::Content => vec![
            "--json".to_string(),
            "--line-number".to_string(),
            "--column".to_string(),
            // The UI currently exposes plain text search, so punctuation must not
            // unexpectedly become a regular-expression operator.
            "--fixed-strings".to_string(),
            "--smart-case".to_string(),
            "--hidden".to_string(),
            "--glob".to_string(),
            "!.git".to_string(),
            "--max-count".to_string(),
            MAX_SEARCH_RESULTS.to_string(),
            "--".to_string(),
            query.to_string(),
            ".".to_string(),
        ],
    }
}

/// Reads both pipes concurrently while the process exits so neither pipe can deadlock the child.
pub(crate) async fn collect_process_output<P, Stdout, Stderr>(
    process: &P,
    stdout: Stdout,
    stderr: Stderr,
) -> Result<(ExitStatus, Vec<u8>, Vec<u8>), WorkspaceFileSystemError>
where
    P: ManagedProcess,
    Stdout: AsyncRead + Unpin,
    Stderr: AsyncRead + Unpin,
{
    let (stdout, stderr, status) =
        tokio::join!(read_bounded(stdout), read_bounded(stderr), process.wait());
    let stdout = stdout.map_err(|source| WorkspaceFileSystemError::SearchFailed {
        message: format!("failed to read ripgrep stdout: {source}"),
    })?;
    let stderr = stderr.map_err(|source| WorkspaceFileSystemError::SearchFailed {
        message: format!("failed to read ripgrep stderr: {source}"),
    })?;
    let status = status.map_err(|source| WorkspaceFileSystemError::SearchFailed {
        message: format!("failed to wait for ripgrep: {source}"),
    })?;
    Ok((status, stdout, stderr))
}

/// Reads one pipe with a sentinel byte so callers can distinguish exact-limit output from overflow.
async fn read_bounded<R>(reader: R) -> std::io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut output = Vec::new();
    reader
        .take((MAX_SEARCH_OUTPUT_BYTES + 1) as u64)
        .read_to_end(&mut output)
        .await?;
    Ok(output)
}

/// Filters ripgrep's file discovery output using a predictable case-insensitive substring match.
fn parse_files(
    root: &CanonicalPathRoot,
    query: &str,
    output: &[u8],
) -> Result<SearchResults, WorkspaceFileSystemError> {
    let query = query.to_lowercase();
    let mut results = Vec::new();
    let mut truncated = false;
    for line in String::from_utf8_lossy(output).lines() {
        let relative = normalize_ripgrep_path(root, line)?;
        if !query.is_empty() && !relative.to_lowercase().contains(&query) {
            continue;
        }
        if results.len() == MAX_SEARCH_RESULTS {
            truncated = true;
            break;
        }
        results.push(SearchResult::File { path: relative });
    }
    Ok(SearchResults { results, truncated })
}

/// Parses only ripgrep JSON match records and ignores summary/context protocol records.
fn parse_matches(
    root: &CanonicalPathRoot,
    output: &[u8],
) -> Result<SearchResults, WorkspaceFileSystemError> {
    let mut results = Vec::new();
    let mut truncated = false;
    for line in output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let message: RipgrepMessage = serde_json::from_slice(line)
            .map_err(|source| WorkspaceFileSystemError::InvalidSearchOutput { source })?;
        let RipgrepMessage::Match { data } = message else {
            continue;
        };
        let Some(first_match) = data.submatches.first() else {
            continue;
        };
        if results.len() == MAX_SEARCH_RESULTS {
            truncated = true;
            break;
        }
        results.push(SearchResult::Match(SearchMatch {
            path: normalize_ripgrep_path(root, &data.path.text)?,
            line: data.line_number,
            column: first_match.start + 1,
            matched_text: first_match.matched.text.clone(),
            preview: data.lines.text.trim_end_matches(['\r', '\n']).to_string(),
        }));
    }
    Ok(SearchResults { results, truncated })
}

/// Normalizes ripgrep's cwd-relative spelling into the crate's slash-separated representation.
pub(crate) fn normalize_ripgrep_path(
    root: &CanonicalPathRoot,
    path: &str,
) -> Result<String, WorkspaceFileSystemError> {
    let path = path.strip_prefix("./").unwrap_or(path);
    relative_string(root, &root.as_path().join(PathBuf::from(path)))
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum RipgrepMessage {
    Match { data: RipgrepMatchData },
    Begin,
    End,
    Context,
    Summary,
}

#[derive(Deserialize)]
struct RipgrepMatchData {
    path: RipgrepText,
    lines: RipgrepText,
    line_number: u64,
    submatches: Vec<RipgrepSubmatch>,
}

#[derive(Deserialize)]
struct RipgrepText {
    text: String,
}

#[derive(Deserialize)]
struct RipgrepSubmatch {
    #[serde(rename = "match")]
    matched: RipgrepText,
    start: u64,
}

#[cfg(test)]
mod tests {
    use super::{SearchKind, SearchMatch, SearchResult, parse_matches, search_arguments};
    use ora_utils::path::CanonicalPathRoot;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    /// Verifies protocol bookkeeping is ignored while line matches retain their location.
    #[test]
    fn parses_ripgrep_json_matches() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        let output = br#"{"type":"begin","data":{"path":{"text":"src/main.rs"}}}
{"type":"match","data":{"path":{"text":"src/main.rs"},"lines":{"text":"fn main() {}\n"},"line_number":3,"absolute_offset":12,"submatches":[{"match":{"text":"main"},"start":3,"end":7}]}}
{"type":"end","data":{"path":{"text":"src/main.rs"},"binary_offset":null,"stats":{}}}
"#;

        let root = CanonicalPathRoot::new(workspace.path())
            .unwrap_or_else(|error| panic!("canonicalize workspace: {error}"));
        let results = parse_matches(&root, output)
            .unwrap_or_else(|error| panic!("parse ripgrep fixture: {error}"));

        assert_eq!(
            results.results,
            vec![SearchResult::Match(SearchMatch {
                path: "src/main.rs".to_string(),
                line: 3,
                column: 4,
                matched_text: "main".to_string(),
                preview: "fn main() {}".to_string(),
            })]
        );
    }

    /// Verifies the plain-text UI search cannot interpret punctuation as regex syntax.
    #[test]
    fn uses_fixed_strings_for_content_search() {
        let arguments = search_arguments("[main]", SearchKind::Content);

        assert!(arguments.contains(&"--fixed-strings".to_string()));
        assert_eq!(arguments.last(), Some(&".".to_string()));
        assert!(arguments.windows(2).any(|pair| pair == ["--", "[main]"]));
    }
}
