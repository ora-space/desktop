use regex::Regex;
use std::error::Error;
use std::sync::LazyLock;

const MAX_RENDERED_CHAIN_DEPTH: usize = 16;
const MAX_CHAIN_TRAVERSAL_DEPTH: usize = 1_024;
const MAX_NODE_CHARS: usize = 512;
const MAX_CHAIN_CHARS: usize = 4_096;
const TRUNCATED: &str = "[truncated]";

static SECRET_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b(authorization|password|passwd|secret|token|api[_-]?key)\b\s*[:=]\s*([^\s,;]+)",
    )
    .unwrap_or_else(|error| panic!("invalid built-in error redaction pattern: {error}"))
});
static URL_CREDENTIAL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(https?://)[^/@\s:]+(?::[^/@\s]*)?@")
        .unwrap_or_else(|error| panic!("invalid built-in URL credential pattern: {error}"))
});
static URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\bhttps?://[^\s,;]+")
        .unwrap_or_else(|error| panic!("invalid built-in URL redaction pattern: {error}"))
});
static WINDOWS_PATH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[a-z]:\\[^\r\n,;]+")
        .unwrap_or_else(|error| panic!("invalid built-in Windows path pattern: {error}"))
});
static POSIX_PATH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(^|[\s(])/(?:[^\s/,;]+/)*[^\s,;)]*")
        .unwrap_or_else(|error| panic!("invalid built-in POSIX path pattern: {error}"))
});
static QUOTED_VALUE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"'[^'\r\n]*'|"[^"\r\n]*""#)
        .unwrap_or_else(|error| panic!("invalid built-in quoted value pattern: {error}"))
});

/// Contains the bounded and sanitized representation of an in-memory error chain.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ErrorReport {
    message: String,
    chain: String,
    chain_depth: usize,
}

impl ErrorReport {
    /// Traverses an error chain once and prepares fields safe for structured runtime logging.
    pub fn from_error(error: &(dyn Error + 'static)) -> Self {
        let mut nodes = Vec::new();
        let mut current = Some(error);
        let mut chain_depth = 0;

        while let Some(node) = current {
            // A custom Error implementation can form a source cycle, so traversal needs a
            // separate hard limit even though only a small prefix is retained for rendering.
            if chain_depth == MAX_CHAIN_TRAVERSAL_DEPTH {
                break;
            }

            chain_depth += 1;
            if nodes.len() < MAX_RENDERED_CHAIN_DEPTH {
                nodes.push(sanitize_node(&node.to_string()));
            }
            current = node.source();
        }

        if current.is_some() || chain_depth > MAX_RENDERED_CHAIN_DEPTH {
            nodes.push(TRUNCATED.to_string());
        }

        let message = nodes
            .first()
            .cloned()
            .unwrap_or_else(|| "unknown error".to_string());
        let chain = truncate_chars(&nodes.join(" <- "), MAX_CHAIN_CHARS);

        Self {
            message,
            chain,
            chain_depth,
        }
    }

    /// Returns the sanitized top-level semantic context.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the single-line, root-to-source diagnostic chain.
    pub fn chain(&self) -> &str {
        &self.chain
    }

    /// Returns the traversed depth, saturated at the absolute source-chain safety limit.
    pub const fn chain_depth(&self) -> usize {
        self.chain_depth
    }
}

fn sanitize_node(value: &str) -> String {
    let single_line = value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>();
    let redacted_secrets = SECRET_PATTERN.replace_all(&single_line, "$1=[REDACTED]");
    let redacted_credentials =
        URL_CREDENTIAL_PATTERN.replace_all(&redacted_secrets, "$1[REDACTED]@");
    let redacted_urls = URL_PATTERN.replace_all(&redacted_credentials, "[URL]");
    let redacted_windows_paths = WINDOWS_PATH_PATTERN.replace_all(&redacted_urls, "[PATH]");
    let redacted_paths = POSIX_PATH_PATTERN.replace_all(&redacted_windows_paths, "$1[PATH]");
    let redacted_values = QUOTED_VALUE_PATTERN.replace_all(&redacted_paths, "[REDACTED]");
    truncate_chars(redacted_values.trim(), MAX_NODE_CHARS)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let keep = max_chars.saturating_sub(TRUNCATED.chars().count() + 1);
    let mut output = value.chars().take(keep).collect::<String>();
    output.push(' ');
    output.push_str(TRUNCATED);
    output
}

#[cfg(test)]
mod tests {
    use super::{ErrorReport, MAX_CHAIN_TRAVERSAL_DEPTH};
    use pretty_assertions::assert_eq;
    use std::fmt;
    use thiserror::Error;

    #[derive(Debug, Error)]
    #[error("database is locked; token=super-secret")]
    struct RootError;

    #[derive(Debug, Error)]
    #[error("failed\nto persist task")]
    struct ContextError {
        #[source]
        source: RootError,
    }

    #[test]
    fn renders_a_single_line_chain_and_redacts_secrets() {
        let report = ErrorReport::from_error(&ContextError { source: RootError });

        assert_eq!(report.message(), "failed to persist task");
        assert_eq!(
            report.chain(),
            "failed to persist task <- database is locked; token=[REDACTED]"
        );
        assert_eq!(report.chain_depth(), 2);
    }

    #[derive(Debug, Error)]
    #[error(
        "git failed at C:\\Users\\alice\\repo; remote=https://alice:secret@example.com/org/repo; value='customer@example.com'"
    )]
    struct SensitiveExternalError;

    #[test]
    fn redacts_paths_remotes_and_quoted_values() {
        let report = ErrorReport::from_error(&SensitiveExternalError);

        assert!(!report.chain().contains("alice"));
        assert!(!report.chain().contains("example.com"));
        assert!(!report.chain().contains("customer"));
        assert!(report.chain().contains("[PATH]"));
        assert!(report.chain().contains("[URL]"));
        assert!(report.chain().contains("[REDACTED]"));
    }

    #[derive(Debug, Error)]
    #[error("{}", "x".repeat(2_000))]
    struct LongExternalError;

    #[test]
    fn marks_oversized_nodes_as_truncated() {
        let report = ErrorReport::from_error(&LongExternalError);

        assert!(report.chain().ends_with("[truncated]"));
        assert!(report.chain().chars().count() <= 512);
    }

    #[derive(Debug)]
    struct CyclicError;

    impl fmt::Display for CyclicError {
        /// Formats the stable node text used to verify bounded cyclic traversal.
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("cyclic error")
        }
    }

    impl std::error::Error for CyclicError {
        /// Deliberately forms an invalid source cycle to exercise the absolute traversal guard.
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(self)
        }
    }

    /// Verifies a malformed cyclic source chain cannot hang completion logging.
    #[test]
    fn stops_traversing_at_the_absolute_source_chain_limit() {
        let report = ErrorReport::from_error(&CyclicError);

        assert_eq!(report.chain_depth(), MAX_CHAIN_TRAVERSAL_DEPTH);
        assert!(report.chain().ends_with("[truncated]"));
    }
}
