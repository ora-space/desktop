//! Renders arbitrary bytes as text without discarding information.
//!
//! `String::from_utf8_lossy` replaces invalid sequences with U+FFFD, after which the original
//! bytes are gone. Diagnostics captured from untrusted producers need the opposite: valid UTF-8
//! stays readable, and every invalid byte is spelled out so the original can be reconstructed.

use std::borrow::Cow;

/// Renders `bytes` as text, escaping only when the input is not valid UTF-8.
///
/// Valid UTF-8 is returned verbatim. Otherwise every invalid byte becomes `\xNN` and every
/// literal backslash becomes `\\`, which keeps the escaped form unambiguous: a reader that knows
/// the text was escaped can recover the exact original byte sequence. The returned flag tells
/// the caller which of the two happened so it can record that fact alongside the text.
pub fn render_bytes_lossless(bytes: &[u8]) -> (Cow<'_, str>, ByteRendering) {
    match std::str::from_utf8(bytes) {
        Ok(text) => (Cow::Borrowed(text), ByteRendering::Utf8),
        Err(_) => (Cow::Owned(escape_bytes(bytes)), ByteRendering::Escaped),
    }
}

/// Reports whether a rendering is the original text or its reversible escaped form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteRendering {
    Utf8,
    Escaped,
}

/// Escapes invalid UTF-8 bytes as `\xNN` and backslashes as `\\`, keeping valid runs verbatim.
fn escape_bytes(mut bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() + 8);
    loop {
        match std::str::from_utf8(bytes) {
            Ok(valid) => {
                push_escaping_backslashes(&mut output, valid);
                return output;
            }
            Err(error) => {
                let (valid, rest) = bytes.split_at(error.valid_up_to());
                // `from_utf8` guarantees the prefix is valid, so this cannot fail.
                push_escaping_backslashes(&mut output, std::str::from_utf8(valid).unwrap_or(""));
                // A `None` error length means the input ended inside a sequence: everything
                // remaining is invalid rather than a bounded run of bad bytes.
                let invalid_len = error.error_len().unwrap_or(rest.len());
                for byte in &rest[..invalid_len] {
                    output.push_str(&format!("\\x{byte:02x}"));
                }
                bytes = &rest[invalid_len..];
            }
        }
    }
}

/// Appends valid text while doubling backslashes so escapes stay distinguishable.
fn push_escaping_backslashes(output: &mut String, text: &str) {
    for character in text.chars() {
        if character == '\\' {
            output.push_str("\\\\");
        } else {
            output.push(character);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ByteRendering, render_bytes_lossless};
    use pretty_assertions::assert_eq;
    use std::borrow::Cow;

    /// Valid UTF-8 is returned untouched, backslashes included.
    #[test]
    fn valid_utf8_is_returned_verbatim() {
        assert_eq!(
            render_bytes_lossless("路径 C:\\logs".as_bytes()),
            (Cow::Borrowed("路径 C:\\logs"), ByteRendering::Utf8)
        );
    }

    /// Invalid bytes are spelled out and backslashes doubled so the original is recoverable.
    #[test]
    fn invalid_bytes_are_escaped_reversibly() {
        let (rendered, rendering) = render_bytes_lossless(b"ok\\\xff\xfe end\xe4\xbd");
        assert_eq!(
            (rendered.as_ref(), rendering),
            ("ok\\\\\\xff\\xfe end\\xe4\\xbd", ByteRendering::Escaped)
        );
    }
}
