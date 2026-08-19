//! Security validation for SVG icons distributed as plugin assets.

use quick_xml::Reader;
use quick_xml::events::BytesStart;
use quick_xml::events::Event;
use thiserror::Error;

/// The size cap for an SVG icon, matching the marketplace packaging limit of 50 KiB.
pub const DEFAULT_MAX_SVG_BYTES: usize = 50 * 1024;

/// Reports the first unsafe construct discovered while validating an SVG.
///
/// The XML stream is checked element by element so the first violation is reported in the same
/// order it appears in the document, giving callers deterministic diagnostics.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SvgValidationError {
    #[error("SVG exceeds the {max} byte security limit")]
    TooLarge { max: usize },
    #[error("SVG is not well-formed XML: {0}")]
    MalformedXml(String),
    #[error("SVG contains the forbidden element `<{element}>`")]
    ForbiddenElement { element: String },
    #[error("SVG element `{element}` references an external resource in `{attribute}`")]
    ExternalReference { element: String, attribute: String },
    #[error("SVG element `{element}` uses the event-handler attribute `{attribute}`")]
    EventAttribute { element: String, attribute: String },
}

/// Validates `svg` bytes against the marketplace icon security policy.
///
/// Rejects oversized files, malformed XML, `<script>` / `<foreignObject>` elements, event-handler
/// attributes, and any `href` / `xlink:href` that targets an external network resource. The
/// check is conservative and fails closed: anything ambiguous is rejected.
pub fn validate(svg: &[u8]) -> Result<(), SvgValidationError> {
    if svg.len() > DEFAULT_MAX_SVG_BYTES {
        return Err(SvgValidationError::TooLarge {
            max: DEFAULT_MAX_SVG_BYTES,
        });
    }

    let mut reader = Reader::from_reader(svg);
    let mut buffer = Vec::new();
    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| SvgValidationError::MalformedXml(error.to_string()))?;
        match event {
            Event::Start(element) | Event::Empty(element) => inspect_start(&element)?,
            Event::Eof => return Ok(()),
            _ => {}
        }
        buffer.clear();
    }
}

/// Inspects one start (or empty) element for forbidden names and unsafe attributes.
fn inspect_start(element: &BytesStart<'_>) -> Result<(), SvgValidationError> {
    let element_name = ascii_lowercase(element.local_name().as_ref());
    let element_spelling = String::from_utf8_lossy(&element_name).into_owned();
    if let Some(forbidden) = forbidden_element(&element_name) {
        return Err(SvgValidationError::ForbiddenElement {
            element: forbidden.into(),
        });
    }

    for attribute in element.attributes() {
        let attribute =
            attribute.map_err(|error| SvgValidationError::MalformedXml(error.to_string()))?;
        let attribute_name = ascii_lowercase(attribute.key.local_name().as_ref());
        let value = attribute
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|error| SvgValidationError::MalformedXml(error.to_string()))?;
        let attribute_spelling = String::from_utf8_lossy(&attribute_name);

        if is_event_handler(&attribute_name) {
            return Err(SvgValidationError::EventAttribute {
                element: element_spelling,
                attribute: attribute_spelling.into_owned(),
            });
        }
        if attribute_name == b"href" && is_external_reference(&value) {
            return Err(SvgValidationError::ExternalReference {
                element: element_spelling,
                attribute: attribute_spelling.into_owned(),
            });
        }
    }
    Ok(())
}

/// Maps an element's lowercase local name to its forbidden public spelling.
fn forbidden_element(element_lower: &[u8]) -> Option<&'static str> {
    match element_lower {
        b"script" => Some("script"),
        b"foreignobject" => Some("foreignObject"),
        _ => None,
    }
}

/// Returns whether an attribute name is an `on*` event handler such as `onload`.
fn is_event_handler(attribute: &[u8]) -> bool {
    attribute.len() >= 3 && attribute.starts_with(b"on") && attribute[2].is_ascii_alphabetic()
}

/// Returns whether an `href` value escapes to an external or script-capable resource.
fn is_external_reference(value: &str) -> bool {
    let value = value.trim();
    value.starts_with("//")
        || value.starts_with("http:")
        || value.starts_with("https:")
        || value.starts_with("javascript:")
        || value.starts_with("data:")
}

/// Lowercases ASCII bytes in place, keeping multi-byte UTF-8 sequences untouched.
fn ascii_lowercase(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(u8::to_ascii_lowercase).collect()
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_MAX_SVG_BYTES, SvgValidationError, validate};
    use pretty_assertions::assert_eq;

    /// Accepts a minimal, well-formed SVG with local fragment references.
    #[test]
    fn accepts_benign_svg() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg"><use href="#icon"/></svg>"##;
        assert_eq!(validate(svg).unwrap(), ());
    }

    /// Rejects an embedded `<script>` element regardless of capitalization.
    #[test]
    fn rejects_script_element() {
        let svg = b"<svg><script>evil()</script></svg>";
        assert_eq!(
            validate(svg).unwrap_err(),
            SvgValidationError::ForbiddenElement {
                element: "script".into()
            }
        );
        assert!(validate(b"<svg><SCRIPT>evil()</SCRIPT></svg>").is_err());
    }

    /// Rejects a `<foreignObject>` element.
    #[test]
    fn rejects_foreign_object() {
        let svg = b"<svg><foreignObject><div>x</div></foreignObject></svg>";
        assert_eq!(
            validate(svg).unwrap_err(),
            SvgValidationError::ForbiddenElement {
                element: "foreignObject".into()
            }
        );
    }

    /// Rejects external resources referenced from `href` and `xlink:href`.
    #[test]
    fn rejects_external_reference() {
        let svg = br#"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="https://evil.example/x.png"/></svg>"#;
        assert!(matches!(
            validate(svg).unwrap_err(),
            SvgValidationError::ExternalReference { .. }
        ));

        let svg = br#"<svg><image href="http://evil.example/x.png"/></svg>"#;
        assert!(matches!(
            validate(svg).unwrap_err(),
            SvgValidationError::ExternalReference { .. }
        ));
    }

    /// Allows fragment links that stay inside the same document.
    #[test]
    fn allows_fragment_reference() {
        let svg = br##"<svg><use href="#icon"/></svg>"##;
        assert_eq!(validate(svg).unwrap(), ());
    }

    /// Rejects event-handler attributes.
    #[test]
    fn rejects_event_handler() {
        let svg = br#"<svg onload="evil()"></svg>"#;
        assert!(matches!(
            validate(svg).unwrap_err(),
            SvgValidationError::EventAttribute { .. }
        ));
    }

    /// Rejects structurally malformed XML instead of guessing an interpretation.
    #[test]
    fn rejects_malformed_xml() {
        let svg = br#"<svg><rect width="10"</svg>"#;
        assert!(matches!(
            validate(svg).unwrap_err(),
            SvgValidationError::MalformedXml(_)
        ));
    }

    /// Rejects a document that exceeds the 50 KiB icon limit.
    #[test]
    fn rejects_oversized_svg() {
        let padding = [b'a'; DEFAULT_MAX_SVG_BYTES];
        let svg = format!("<svg>{}</svg>", String::from_utf8_lossy(&padding));
        assert_eq!(
            validate(svg.as_bytes()).unwrap_err(),
            SvgValidationError::TooLarge {
                max: DEFAULT_MAX_SVG_BYTES
            }
        );
    }
}
