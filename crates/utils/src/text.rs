//! Generic text normalization utilities.

use std::borrow::Cow;

/// Normalizes Windows CRLF line endings to LF while preserving all other text exactly.
pub fn normalize_newlines(source: &str) -> Cow<'_, str> {
    if source.contains("\r\n") {
        Cow::Owned(source.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(source)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_newlines;
    use pretty_assertions::assert_eq;

    /// CRLF is the only representation normalized so other control characters remain observable.
    #[test]
    fn normalizes_only_crlf_line_endings() {
        assert_eq!(
            normalize_newlines("first\r\nsecond\nthird\rfourth"),
            "first\nsecond\nthird\rfourth"
        );
    }
}
