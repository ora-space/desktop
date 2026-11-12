//! Minimal workspace-relative glob matching.
//!
//! The supported syntax is deliberately small so a declaration stays explainable across
//! platforms: `*` matches within one path segment, `**` as a complete segment matches zero or
//! more segments, and `?` matches one character. Character classes, alternation, escaping and
//! negation are rejected rather than treated as literals, because accepting a pattern whose
//! meaning differs between filesystems would make a completion gate disagree with itself.

use super::StrictRelativePath;
use std::fmt;
use thiserror::Error;

/// Reports why one declared glob is outside the supported syntax or path shape.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RelativeGlobError {
    #[error("glob must not be empty")]
    Empty,
    #[error("glob must not contain a backslash; use '/' as the separator")]
    Backslash,
    #[error("glob must not use the unsupported pattern character {character:?}")]
    UnsupportedCharacter { character: char },
    #[error("glob negation is not supported")]
    Negation,
    #[error("glob '**' must be a complete path segment")]
    InvalidGlobstar,
    #[error("glob is not a workspace-relative path or contains an unsafe segment")]
    UnsafePath,
}

/// One validated workspace-relative glob with a stable slash-separated spelling.
///
/// Matching is case-sensitive and works on the portable `/`-separated form only; it performs no
/// Unicode normalization and does not touch the filesystem. Callers that need to verify a match
/// stays inside a workspace root check containment separately.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RelativeGlob {
    pattern: String,
    segments: Vec<String>,
}

impl RelativeGlob {
    /// Parses one declared glob, reusing [`StrictRelativePath`] for the literal path shape.
    pub fn parse(pattern: &str) -> Result<Self, RelativeGlobError> {
        if pattern.is_empty() {
            return Err(RelativeGlobError::Empty);
        }
        if pattern.contains('\\') {
            return Err(RelativeGlobError::Backslash);
        }
        if pattern.starts_with('!') {
            return Err(RelativeGlobError::Negation);
        }
        for character in ['[', ']', '{', '}', '\u{7f}'] {
            if pattern.contains(character) {
                return Err(RelativeGlobError::UnsupportedCharacter { character });
            }
        }
        // Strict parsing rejects absolute paths, `..`, empty and `.` segments, control
        // characters, and platform-reserved names; glob metacharacters are ordinary literal
        // characters there, so the path shape is validated without weakening the pattern.
        let path = StrictRelativePath::parse(pattern).map_err(|_| RelativeGlobError::UnsafePath)?;
        let segments: Vec<String> = path.as_str().split('/').map(str::to_string).collect();
        for segment in &segments {
            if segment.contains("**") && segment != "**" {
                return Err(RelativeGlobError::InvalidGlobstar);
            }
        }
        Ok(Self {
            pattern: path.as_str().to_string(),
            segments,
        })
    }

    /// Returns the validated slash-separated pattern exactly as declared.
    pub fn as_str(&self) -> &str {
        &self.pattern
    }

    /// Returns whether one workspace-relative POSIX path matches this glob.
    ///
    /// The path is split on `/` without normalization, so a caller must pass the same portable
    /// spelling it would persist. `**` spans zero or more segments and `*` never crosses a `/`.
    pub fn matches(&self, path: &str) -> bool {
        let path_segments: Vec<&str> = path.split('/').collect();
        let mut memo = vec![vec![None; path_segments.len() + 1]; self.segments.len() + 1];
        matches_segments(&self.segments, 0, &path_segments, 0, &mut memo)
    }
}

impl fmt::Display for RelativeGlob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.pattern)
    }
}

/// Matches the remaining glob segments against the remaining path segments, memoizing both
/// indices so repeated `**` branching cannot degenerate into exponential backtracking.
fn matches_segments(
    pattern: &[String],
    pattern_index: usize,
    path: &[&str],
    path_index: usize,
    memo: &mut [Vec<Option<bool>>],
) -> bool {
    if let Some(cached) = memo[pattern_index][path_index] {
        return cached;
    }
    let result = if pattern_index == pattern.len() {
        path_index == path.len()
    } else if pattern[pattern_index] == "**" {
        // Zero segments, or consume one more path segment and keep the globstar available.
        matches_segments(pattern, pattern_index + 1, path, path_index, memo)
            || (path_index < path.len()
                && matches_segments(pattern, pattern_index, path, path_index + 1, memo))
    } else {
        path_index < path.len()
            && matches_segment(&pattern[pattern_index], path[path_index])
            && matches_segments(pattern, pattern_index + 1, path, path_index + 1, memo)
    };
    memo[pattern_index][path_index] = Some(result);
    result
}

/// Matches one path segment against a pattern segment supporting `*` and `?`.
fn matches_segment(pattern: &str, candidate: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let candidate: Vec<char> = candidate.chars().collect();
    let mut memo = vec![vec![false; candidate.len() + 1]; pattern.len() + 1];
    memo[pattern.len()][candidate.len()] = true;
    for pattern_index in (0..=pattern.len()).rev() {
        for candidate_index in (0..=candidate.len()).rev() {
            if pattern_index == pattern.len() {
                memo[pattern_index][candidate_index] = candidate_index == candidate.len();
                continue;
            }
            memo[pattern_index][candidate_index] = match pattern[pattern_index] {
                '*' => {
                    memo[pattern_index + 1][candidate_index]
                        || (candidate_index < candidate.len()
                            && memo[pattern_index][candidate_index + 1])
                }
                '?' => {
                    candidate_index < candidate.len()
                        && memo[pattern_index + 1][candidate_index + 1]
                }
                literal => {
                    candidate_index < candidate.len()
                        && literal == candidate[candidate_index]
                        && memo[pattern_index + 1][candidate_index + 1]
                }
            };
        }
    }
    memo[0][0]
}

#[cfg(test)]
mod tests {
    use super::{RelativeGlob, RelativeGlobError};
    use pretty_assertions::assert_eq;

    /// Parses one pattern, failing the test with the pattern that was rejected.
    fn glob(pattern: &str) -> RelativeGlob {
        RelativeGlob::parse(pattern).unwrap_or_else(|error| panic!("parse {pattern:?}: {error}"))
    }

    /// Verifies every supported wildcard shape is accepted.
    #[test]
    fn accepts_the_supported_wildcards() {
        for pattern in [
            "reports/plan.md",
            "reports/*.md",
            "reports/**/*.md",
            "reports/?.md",
            "**/*.md",
            "reports/**",
            "a/*/b/*.md",
        ] {
            assert_eq!(glob(pattern).as_str(), pattern);
        }
    }

    /// Verifies path escapes and unsupported glob syntax are refused instead of interpreted.
    #[test]
    fn rejects_unsafe_paths_and_unsupported_syntax() {
        let cases = [
            ("", RelativeGlobError::Empty),
            ("/absolute/*.md", RelativeGlobError::UnsafePath),
            ("../outside.md", RelativeGlobError::UnsafePath),
            ("a//b.md", RelativeGlobError::UnsafePath),
            ("a/./b.md", RelativeGlobError::UnsafePath),
            ("a\\b.md", RelativeGlobError::Backslash),
            (
                "a/[0-9].md",
                RelativeGlobError::UnsupportedCharacter { character: '[' },
            ),
            (
                "a/{one,two}.md",
                RelativeGlobError::UnsupportedCharacter { character: '{' },
            ),
            ("a/**b/c.md", RelativeGlobError::InvalidGlobstar),
            ("a/***/c.md", RelativeGlobError::InvalidGlobstar),
        ];
        for (pattern, expected) in cases {
            assert_eq!(RelativeGlob::parse(pattern), Err(expected), "{pattern}");
        }
    }

    /// Verifies `*` and `?` stay inside one segment while `**` spans directories.
    #[test]
    fn wildcards_keep_their_segment_boundaries() {
        let star = glob("reports/*.md");
        assert!(star.matches("reports/plan.md"));
        assert!(!star.matches("reports/nested/plan.md"));
        assert!(!star.matches("Reports/plan.md"));

        let question = glob("reports/?.md");
        assert!(question.matches("reports/a.md"));
        assert!(!question.matches("reports/ab.md"));
        assert!(!question.matches("reports/.md"));

        let globstar = glob("reports/**/*.md");
        assert!(globstar.matches("reports/plan.md"));
        assert!(globstar.matches("reports/nested/plan.md"));
        assert!(globstar.matches("reports/a/b/plan.md"));
        assert!(!globstar.matches("other/plan.md"));
    }

    /// A leading `**` matches files at the root and below, never a sibling directory.
    #[test]
    fn leading_globstar_matches_zero_or_more_directories() {
        let all_markdown = glob("**/*.md");
        assert!(all_markdown.matches("README.md"));
        assert!(all_markdown.matches("docs/README.md"));
        assert!(all_markdown.matches("docs/nested/README.md"));
        assert!(!all_markdown.matches("docs/README.txt"));
    }

    /// Matching is case-sensitive and makes no Unicode normalization decision.
    #[test]
    fn matching_is_case_sensitive_without_unicode_normalization() {
        let lower = glob("docs/report.md");
        assert!(!lower.matches("docs/Report.md"));

        let composed = glob("docs/caf\u{e9}.md");
        assert!(composed.matches("docs/caf\u{e9}.md"));
        assert!(!composed.matches("docs/cafe\u{301}.md"));

        let question = glob("docs/?.md");
        assert!(question.matches("docs/\u{e9}.md"));
        assert!(!question.matches("docs/e\u{301}.md"));
    }
}
