//! Rejects user-authored text that embeds scriptable (unfiltered) HTML.

use thiserror::Error;

/// HTML tags whose presence in marketplace README text makes it unsafe to render.
const FORBIDDEN_TAGS: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "base", "form", "link", "meta", "template",
];

/// Reports the first unsafe HTML construct discovered while validating README text.
///
/// Validation stops at the first offence in a stable order so callers get deterministic errors.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum HtmlValidationError {
    #[error("README embeds the forbidden HTML tag `{tag}`")]
    ForbiddenTag { tag: &'static str },
    #[error("README uses the event-handler attribute `{attribute}`")]
    EventAttribute { attribute: String },
    #[error("README uses `{attribute}` with a script-capable URI")]
    UnsafeUri { attribute: String },
}

/// Returns `Ok(())` when `source` contains no scriptable HTML.
///
/// The scan is intentionally conservative and fails closed: any occurrence of a forbidden tag,
/// an `on*` event-handler attribute, or a `javascript:` / `data:` / `vbscript:` URI rejects the
/// whole text, even inside comments or code fences, because the caller may render it as HTML.
pub fn validate(source: &str) -> Result<(), HtmlValidationError> {
    let lowercase = source.to_ascii_lowercase();
    for tag in FORBIDDEN_TAGS {
        if contains_tag(&lowercase, tag) {
            return Err(HtmlValidationError::ForbiddenTag { tag });
        }
    }
    scan_tags(&lowercase)?;
    Ok(())
}

/// Detects an open or close form of `tag` whose next character is a real tag boundary.
fn contains_tag(source: &str, tag: &str) -> bool {
    let open = format!("<{tag}");
    let close = format!("</{tag}");
    contains_delimited(source, &open) || contains_delimited(source, &close)
}

/// Returns whether `needle` occurs in `source` immediately followed by a tag boundary.
fn contains_delimited(source: &str, needle: &str) -> bool {
    let haystack = source.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    let mut index = 0;
    while index + needle.len() <= haystack.len() {
        if &haystack[index..index + needle.len()] == needle {
            let following = haystack.get(index + needle.len());
            let bounded = following.is_none_or(|byte| {
                matches!(
                    byte,
                    b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>' | b'"' | b'\''
                )
            });
            if bounded {
                return true;
            }
            index += needle.len();
        } else {
            index += 1;
        }
    }
    false
}

/// Inspects each `<…>` tag body for event handlers and script-capable attribute values.
fn scan_tags(source: &str) -> Result<(), HtmlValidationError> {
    let mut remaining = source;
    while let Some(offset) = remaining.find('<') {
        let after_open = &remaining[offset + 1..];
        let Some(close) = after_open.find('>') else {
            break;
        };
        scan_tag(&after_open[..close])?;
        remaining = &after_open[close + 1..];
    }
    Ok(())
}

/// Rejects event-handler attributes and script-capable URIs inside a single tag body.
fn scan_tag(tag: &str) -> Result<(), HtmlValidationError> {
    for token in tag.split_whitespace() {
        let attribute = token.to_ascii_lowercase();
        let Some((name, raw_value)) = attribute.split_once('=') else {
            continue;
        };
        let value = raw_value.trim_matches(|c| matches!(c, '"' | '\'')).trim();
        if is_event_handler(name) {
            return Err(HtmlValidationError::EventAttribute {
                attribute: name.into(),
            });
        }
        if is_script_uri(value) {
            return Err(HtmlValidationError::UnsafeUri {
                attribute: name.into(),
            });
        }
    }
    Ok(())
}

/// Returns whether an attribute name is an `on*` event handler such as `onload`.
fn is_event_handler(attribute: &str) -> bool {
    let bytes = attribute.as_bytes();
    bytes.len() >= 3 && bytes.starts_with(b"on") && bytes[2].is_ascii_alphabetic()
}

/// Returns whether a URL value enables script execution through its scheme.
fn is_script_uri(value: &str) -> bool {
    value.starts_with("javascript:") || value.starts_with("data:") || value.starts_with("vbscript:")
}

#[cfg(test)]
mod tests {
    use super::{HtmlValidationError, validate};
    use pretty_assertions::assert_eq;

    /// Accepts plain prose and benign inline markup.
    #[test]
    fn accepts_benign_markup() {
        assert_eq!(validate("A weather plugin for Oracle.").unwrap(), ());
        assert_eq!(validate("Use the **bold** and `code`.").unwrap(), ());
        assert_eq!(
            validate("<em>tip</em> and <code>config</code>").unwrap(),
            ()
        );
    }

    /// Rejects a `<script>` tag regardless of case or attribute spacing.
    #[test]
    fn rejects_script_tag() {
        let error = validate("<p>hi</p><script>alert(1)</script>").unwrap_err();
        assert_eq!(error, HtmlValidationError::ForbiddenTag { tag: "script" });
        assert!(validate("<SCRIPT>alert(1)</SCRIPT>").is_err());
    }

    /// Rejects an `on*` event-handler attribute.
    #[test]
    fn rejects_event_handler() {
        let error = validate("<img src=\"x\" onerror=\"alert(1)\">").unwrap_err();
        assert_eq!(
            error,
            HtmlValidationError::EventAttribute {
                attribute: "onerror".into()
            }
        );
    }

    /// Rejects `javascript:` and `data:` URIs in attributes.
    #[test]
    fn rejects_script_uris() {
        assert!(validate("<a href=\"javascript:void(0)\">x</a>").is_err());
        assert!(validate("<img src=\"data:text/html;base64,PHNjcmlwdD4=\">").is_err());
    }

    /// Returns the first forbidden tag in a stable, deterministic order.
    #[test]
    fn first_forbidden_tag_is_deterministic() {
        let error = validate("<iframe></iframe><script>x()</script>").unwrap_err();
        assert_eq!(error, HtmlValidationError::ForbiddenTag { tag: "script" });
    }
}
