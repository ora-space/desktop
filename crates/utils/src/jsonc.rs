//! Comment-preserving edits for one entry nested in a JSON/JSONC object.

use serde_json::Value;
use std::ops::Range;
use thiserror::Error;

/// Reports malformed JSONC or a requested object path whose existing value is not an object.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum JsoncEditError {
    #[error("invalid JSONC: {0}")]
    Invalid(String),
    #[error("JSONC member `{0}` must be an object")]
    ExpectedObject(String),
}

/// Replaces or inserts one entry below a top-level object while preserving unrelated source text.
pub fn set_nested_object_entry(
    source: &str,
    object_key: &str,
    entry_key: &str,
    value: &Value,
) -> Result<String, JsoncEditError> {
    let source = if source.trim().is_empty() {
        "{}"
    } else {
        source
    };
    let root = parse_object(source, first_non_trivia(source, 0)?)?;
    let rendered = serde_json::to_string_pretty(value)
        .map_err(|error| JsoncEditError::Invalid(error.to_string()))?;
    if let Some(parent) = root.members.iter().find(|member| member.key == object_key) {
        let parent_object = parse_object(source, first_non_trivia(source, parent.value.start)?)
            .map_err(|_| JsoncEditError::ExpectedObject(object_key.to_string()))?;
        if let Some(existing) = parent_object
            .members
            .iter()
            .find(|member| member.key == entry_key)
        {
            return Ok(replace_range(source, existing.value.clone(), &rendered));
        }
        return insert_member(source, &parent_object, entry_key, &rendered);
    }
    let nested = format!(
        "{{\n    {}: {}\n  }}",
        serde_json::to_string(entry_key)
            .map_err(|error| JsoncEditError::Invalid(error.to_string()))?,
        indent_after_first(&rendered, 4)
    );
    insert_member(source, &root, object_key, &nested)
}

/// Removes one nested entry without reformatting unrelated keys or comments.
pub fn remove_nested_object_entry(
    source: &str,
    object_key: &str,
    entry_key: &str,
) -> Result<String, JsoncEditError> {
    let root = parse_object(source, first_non_trivia(source, 0)?)?;
    let Some(parent) = root.members.iter().find(|member| member.key == object_key) else {
        return Ok(source.to_string());
    };
    let parent_object = parse_object(source, first_non_trivia(source, parent.value.start)?)
        .map_err(|_| JsoncEditError::ExpectedObject(object_key.to_string()))?;
    let Some(index) = parent_object
        .members
        .iter()
        .position(|member| member.key == entry_key)
    else {
        return Ok(source.to_string());
    };
    let member = &parent_object.members[index];
    let removal = if index + 1 < parent_object.members.len() {
        member.full.start..parent_object.members[index + 1].full.start
    } else if index > 0 {
        parent_object.members[index - 1].value.end..member.full.end
    } else {
        member.full.clone()
    };
    Ok(replace_range(source, removal, ""))
}

/// Parses JSONC into semantic JSON for conflict checks while accepting comments and trailing commas.
pub fn parse_value(source: &str) -> Result<Value, JsoncEditError> {
    let without_comments = remove_comments(source)?;
    let without_trailing_commas = remove_trailing_commas(&without_comments);
    serde_json::from_str(&without_trailing_commas)
        .map_err(|error| JsoncEditError::Invalid(error.to_string()))
}

/// Returns one nested value when both path segments select objects.
pub fn nested_value<'a>(value: &'a Value, object_key: &str, entry_key: &str) -> Option<&'a Value> {
    value.get(object_key)?.as_object()?.get(entry_key)
}

#[derive(Clone, Debug)]
struct ObjectSpan {
    close: usize,
    members: Vec<MemberSpan>,
}

#[derive(Clone, Debug)]
struct MemberSpan {
    key: String,
    full: Range<usize>,
    value: Range<usize>,
}

/// Locates one object and its direct members without interpreting nested values.
fn parse_object(source: &str, start: usize) -> Result<ObjectSpan, JsoncEditError> {
    if source.as_bytes().get(start) != Some(&b'{') {
        return Err(JsoncEditError::Invalid("expected an object".to_string()));
    }
    let mut cursor = start + 1;
    let mut members = Vec::new();
    loop {
        cursor = first_non_trivia(source, cursor)?;
        if source.as_bytes().get(cursor) == Some(&b'}') {
            return Ok(ObjectSpan {
                close: cursor,
                members,
            });
        }
        let member_start = cursor;
        let (key, after_key) = parse_string(source, cursor)?;
        cursor = first_non_trivia(source, after_key)?;
        if source.as_bytes().get(cursor) != Some(&b':') {
            return Err(JsoncEditError::Invalid(format!(
                "expected `:` after object key at byte {cursor}"
            )));
        }
        let value_start = first_non_trivia(source, cursor + 1)?;
        let value_end = skip_value(source, value_start)?;
        let mut after_value = first_non_trivia(source, value_end)?;
        let full_end = match source.as_bytes().get(after_value) {
            Some(b',') => {
                after_value += 1;
                after_value
            }
            Some(b'}') => after_value,
            _ => {
                return Err(JsoncEditError::Invalid(format!(
                    "expected `,` or `}}` at byte {after_value}"
                )));
            }
        };
        members.push(MemberSpan {
            key,
            full: member_start..full_end,
            value: value_start..value_end,
        });
        cursor = after_value;
    }
}

/// Inserts a member immediately before an object's closing brace using stable two-space layout.
fn insert_member(
    source: &str,
    object: &ObjectSpan,
    key: &str,
    rendered: &str,
) -> Result<String, JsoncEditError> {
    let key =
        serde_json::to_string(key).map_err(|error| JsoncEditError::Invalid(error.to_string()))?;
    let prefix = if object.members.is_empty() {
        "\n"
    } else if object
        .members
        .last()
        .is_some_and(|member| source[member.full.clone()].trim_end().ends_with(','))
    {
        // JSONC commonly leaves a trailing comma before comments or the closing brace. Adding a
        // second separator would make the edited document invalid, so that comma is reused.
        ""
    } else {
        ",\n"
    };
    let insertion = format!("{prefix}  {key}: {}\n", indent_after_first(rendered, 2));
    Ok(replace_range(
        source,
        object.close..object.close,
        &insertion,
    ))
}

/// Skips one complete JSONC value and returns the first byte after it.
fn skip_value(source: &str, start: usize) -> Result<usize, JsoncEditError> {
    match source.as_bytes().get(start).copied() {
        Some(b'"') => parse_string(source, start).map(|(_, end)| end),
        Some(b'{') => skip_balanced(source, start, b'{', b'}'),
        Some(b'[') => skip_balanced(source, start, b'[', b']'),
        Some(_) => {
            let mut cursor = start;
            while let Some(byte) = source.as_bytes().get(cursor) {
                if matches!(byte, b',' | b'}' | b']') || byte.is_ascii_whitespace() {
                    break;
                }
                cursor += 1;
            }
            if cursor == start {
                Err(JsoncEditError::Invalid(format!(
                    "expected a value at byte {start}"
                )))
            } else {
                Ok(cursor)
            }
        }
        None => Err(JsoncEditError::Invalid(
            "unexpected end of input".to_string(),
        )),
    }
}

/// Skips a nested object or array while honoring strings and comments.
fn skip_balanced(source: &str, start: usize, open: u8, close: u8) -> Result<usize, JsoncEditError> {
    let mut cursor = start;
    let mut depth = 0_u32;
    while let Some(byte) = source.as_bytes().get(cursor).copied() {
        match byte {
            b'"' => cursor = parse_string(source, cursor)?.1,
            b'/' if source.as_bytes().get(cursor + 1) == Some(&b'/') => {
                cursor = skip_line_comment(source, cursor + 2);
            }
            b'/' if source.as_bytes().get(cursor + 1) == Some(&b'*') => {
                cursor = skip_block_comment(source, cursor + 2)?;
            }
            byte if byte == open => {
                depth += 1;
                cursor += 1;
            }
            byte if byte == close => {
                depth = depth.checked_sub(1).ok_or_else(|| {
                    JsoncEditError::Invalid(format!("unmatched delimiter at byte {cursor}"))
                })?;
                cursor += 1;
                if depth == 0 {
                    return Ok(cursor);
                }
            }
            _ => cursor += 1,
        }
    }
    Err(JsoncEditError::Invalid("unterminated value".to_string()))
}

/// Decodes a JSON string token and returns its end offset.
fn parse_string(source: &str, start: usize) -> Result<(String, usize), JsoncEditError> {
    let mut cursor = start + 1;
    let mut escaped = false;
    while let Some(byte) = source.as_bytes().get(cursor).copied() {
        if escaped {
            escaped = false;
            cursor += 1;
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'"' => {
                let end = cursor + 1;
                let decoded = serde_json::from_str(&source[start..end])
                    .map_err(|error| JsoncEditError::Invalid(error.to_string()))?;
                return Ok((decoded, end));
            }
            _ => {}
        }
        cursor += 1;
    }
    Err(JsoncEditError::Invalid("unterminated string".to_string()))
}

/// Advances past whitespace and comments.
fn first_non_trivia(source: &str, mut cursor: usize) -> Result<usize, JsoncEditError> {
    loop {
        while source
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        match (
            source.as_bytes().get(cursor),
            source.as_bytes().get(cursor + 1),
        ) {
            (Some(b'/'), Some(b'/')) => cursor = skip_line_comment(source, cursor + 2),
            (Some(b'/'), Some(b'*')) => cursor = skip_block_comment(source, cursor + 2)?,
            _ => return Ok(cursor),
        }
    }
}

fn skip_line_comment(source: &str, mut cursor: usize) -> usize {
    while source
        .as_bytes()
        .get(cursor)
        .is_some_and(|byte| *byte != b'\n')
    {
        cursor += 1;
    }
    cursor
}

fn skip_block_comment(source: &str, mut cursor: usize) -> Result<usize, JsoncEditError> {
    while cursor + 1 < source.len() {
        if &source.as_bytes()[cursor..cursor + 2] == b"*/" {
            return Ok(cursor + 2);
        }
        cursor += 1;
    }
    Err(JsoncEditError::Invalid(
        "unterminated block comment".to_string(),
    ))
}

/// Removes comments while retaining byte separation so adjacent tokens cannot collapse.
fn remove_comments(source: &str) -> Result<String, JsoncEditError> {
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0;
    while cursor < source.len() {
        match (
            source.as_bytes().get(cursor),
            source.as_bytes().get(cursor + 1),
        ) {
            (Some(b'"'), _) => {
                let end = parse_string(source, cursor)?.1;
                output.push_str(&source[cursor..end]);
                cursor = end;
            }
            (Some(b'/'), Some(b'/')) => {
                cursor = skip_line_comment(source, cursor + 2);
                output.push(' ');
            }
            (Some(b'/'), Some(b'*')) => {
                cursor = skip_block_comment(source, cursor + 2)?;
                output.push(' ');
            }
            _ => {
                output.push(source.as_bytes()[cursor] as char);
                cursor += 1;
            }
        }
    }
    Ok(output)
}

/// Drops commas whose next non-whitespace token closes an object or array.
fn remove_trailing_commas(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut cursor = 0;
    let mut in_string = false;
    let mut escaped = false;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        if in_string {
            output.push(byte as char);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            cursor += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
        }
        if byte == b',' {
            let mut lookahead = cursor + 1;
            while bytes.get(lookahead).is_some_and(u8::is_ascii_whitespace) {
                lookahead += 1;
            }
            if matches!(bytes.get(lookahead), Some(b'}' | b']')) {
                cursor += 1;
                continue;
            }
        }
        output.push(byte as char);
        cursor += 1;
    }
    output
}

fn replace_range(source: &str, range: Range<usize>, replacement: &str) -> String {
    let mut output = String::with_capacity(source.len() - range.len() + replacement.len());
    output.push_str(&source[..range.start]);
    output.push_str(replacement);
    output.push_str(&source[range.end..]);
    output
}

fn indent_after_first(value: &str, spaces: usize) -> String {
    let indentation = " ".repeat(spaces);
    value.replace('\n', &format!("\n{indentation}"))
}

#[cfg(test)]
mod tests {
    use super::{parse_value, remove_nested_object_entry, set_nested_object_entry};
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Edits a managed nested key without changing unrelated comments or trailing commas.
    #[test]
    fn preserves_jsonc_around_nested_updates() {
        let source = "{\n  // keep\n  \"theme\": \"dark\",\n  \"mcp\": {\n    \"user\": {\"type\":\"remote\"},\n  },\n}\n";
        let updated = set_nested_object_entry(
            source,
            "mcp",
            "ora",
            &json!({"type":"remote","url":"https://example.com"}),
        )
        .expect("set nested entry");

        assert!(updated.contains("// keep"));
        assert!(updated.contains("\"theme\": \"dark\""));
        assert_eq!(
            parse_value(&updated).expect("parse updated JSONC")["mcp"]["ora"],
            json!({"type":"remote","url":"https://example.com"})
        );

        let removed =
            remove_nested_object_entry(&updated, "mcp", "ora").expect("remove nested entry");
        assert!(removed.contains("// keep"));
        assert_eq!(
            parse_value(&removed).expect("parse removed JSONC")["mcp"]["user"],
            json!({"type":"remote"})
        );
    }
}
