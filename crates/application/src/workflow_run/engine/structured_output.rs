use crate::workflow_run::engine::variable_value::{
    is_supported_variable_type, workflow_value_matches_type,
};
use serde_json::Value;
use thiserror::Error;

/// Failures raised while parsing and validating an agent node's structured output.
///
/// The validator intentionally covers the subset of JSON Schema the workflow editor emits —
/// `type`, `properties`, `required`, `additionalProperties`, and `items` — rather than pulling in
/// a full JSON Schema implementation, keeping the executor dependency-light.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum StructuredOutputError {
    #[error("final assistant output is not a JSON object: {reason}")]
    NotJsonObject { reason: String },
    #[error("structured output at {path} has type {actual}, expected {expected}")]
    InvalidType {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("structured output at {path} is missing required property {property}")]
    MissingRequired { path: String, property: String },
    #[error("structured output at {path} has unexpected property {property}")]
    DisallowedProperty { path: String, property: String },
    #[error("structured output at {path} violates the schema: {message}")]
    SchemaViolation { path: String, message: String },
}

/// Extracts a JSON object from an agent's final response.
///
/// Tries, in order: a whole response that already is a bare object, the first fence body that
/// parses as an object, and finally the first balanced JSON object embedded anywhere in the text.
/// The last fallback tolerates the common model habit of wrapping the object in prose instead of a
/// fence; the schema validator still gates the recovered value's shape.
pub fn extract_json_object(text: &str) -> Result<Value, StructuredOutputError> {
    let trimmed = text.trim();
    // A bare object may still be followed by commentary, so a whole-response parse failure falls
    // through to fence and inline recovery instead of rejecting immediately.
    if trimmed.starts_with('{')
        && let Ok(value) = serde_json::from_str::<Value>(trimmed)
        && value.is_object()
    {
        return Ok(value);
    }
    for block in fenced_blocks(trimmed) {
        if let Ok(value) = serde_json::from_str::<Value>(&block)
            && value.is_object()
        {
            return Ok(value);
        }
    }
    if let Some(value) = first_embedded_json_object(trimmed) {
        return Ok(value);
    }
    Err(StructuredOutputError::NotJsonObject {
        reason: if trimmed.starts_with('{') {
            "text is not valid JSON".to_string()
        } else {
            "no JSON object found in the final response".to_string()
        },
    })
}

/// Locates the first balanced JSON object embedded anywhere in `text`.
///
/// Each `{` is a candidate start; the object ends at the `}` that closes it at depth zero, ignoring
/// braces inside quoted strings. Only candidates that actually parse as an object are returned, so
/// prose mentioning JSON is skipped and a real embedded object still wins.
fn first_embedded_json_object(text: &str) -> Option<Value> {
    for (offset, ch) in text.char_indices() {
        if ch != '{' {
            continue;
        }
        let candidate = &text[offset..];
        let Some(end) = object_body_end(candidate) else {
            continue;
        };
        if let Ok(value) = serde_json::from_str::<Value>(&candidate[..end])
            && value.is_object()
        {
            return Some(value);
        }
    }
    None
}

/// Byte length of the balanced JSON object that begins at the start of `text`, if one closes.
fn object_body_end(text: &str) -> Option<usize> {
    let mut depth = 0i64;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(offset + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

/// Validates a parsed value against a JSON Schema subset, reporting the first violation.
pub fn validate_against_schema(value: &Value, schema: &Value) -> Result<(), StructuredOutputError> {
    validate_schema_node(schema, "$")?;
    validate_node(value, schema, "$")
}

/// Validates a persisted Agent structured-output contract before execution begins.
pub(super) fn validate_structured_output_schema(
    schema: &Value,
) -> Result<(), StructuredOutputError> {
    validate_schema_node(schema, "$")?;
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err(StructuredOutputError::SchemaViolation {
            path: "$".to_string(),
            message: "root type must be object".to_string(),
        });
    }
    Ok(())
}

/// Validates the supported Schema subset before using it to accept Agent output.
fn validate_schema_node(schema: &Value, path: &str) -> Result<(), StructuredOutputError> {
    let object = schema
        .as_object()
        .ok_or_else(|| StructuredOutputError::SchemaViolation {
            path: path.to_string(),
            message: "schema fragment is not an object".to_string(),
        })?;
    let value_type = object.get("type").and_then(Value::as_str).ok_or_else(|| {
        StructuredOutputError::SchemaViolation {
            path: path.to_string(),
            message: "type must be a string".to_string(),
        }
    })?;
    if value_type != "null" && !is_supported_variable_type(value_type) {
        return Err(StructuredOutputError::SchemaViolation {
            path: path.to_string(),
            message: format!("unsupported type {value_type}"),
        });
    }
    if let Some(properties) = object.get("properties") {
        if value_type != "object" {
            return Err(StructuredOutputError::SchemaViolation {
                path: path.to_string(),
                message: "properties is only valid for object fields".to_string(),
            });
        }
        let properties =
            properties
                .as_object()
                .ok_or_else(|| StructuredOutputError::SchemaViolation {
                    path: path.to_string(),
                    message: "properties must be an object".to_string(),
                })?;
        for (name, property) in properties {
            validate_schema_node(property, &format!("{path}.properties.{name}"))?;
        }
    }
    if let Some(required) = object.get("required") {
        if value_type != "object" {
            return Err(StructuredOutputError::SchemaViolation {
                path: path.to_string(),
                message: "required is only valid for object fields".to_string(),
            });
        }
        let required =
            required
                .as_array()
                .ok_or_else(|| StructuredOutputError::SchemaViolation {
                    path: path.to_string(),
                    message: "required must be an array".to_string(),
                })?;
        for entry in required {
            let name = entry
                .as_str()
                .ok_or_else(|| StructuredOutputError::SchemaViolation {
                    path: path.to_string(),
                    message: "required entry is not a string".to_string(),
                })?;
            if !object
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|properties| properties.contains_key(name))
            {
                return Err(StructuredOutputError::SchemaViolation {
                    path: path.to_string(),
                    message: format!("required property {name} is not declared"),
                });
            }
        }
    }
    if let Some(additional) = object.get("additionalProperties")
        && (value_type != "object" || !additional.is_boolean())
    {
        return Err(StructuredOutputError::SchemaViolation {
            path: path.to_string(),
            message: "additionalProperties must be a boolean on an object field".to_string(),
        });
    }
    if let Some(items) = object.get("items") {
        if value_type != "array" {
            return Err(StructuredOutputError::SchemaViolation {
                path: path.to_string(),
                message: "items is only valid for array fields".to_string(),
            });
        }
        validate_schema_node(items, &format!("{path}.items"))?;
    }
    Ok(())
}

/// Recursively validates one node against its schema fragment, tracking a JSON-pointer-style path.
fn validate_node(value: &Value, schema: &Value, path: &str) -> Result<(), StructuredOutputError> {
    if let Some(expected) = schema.get("type").and_then(Value::as_str) {
        let actual = type_label(value);
        if !matches_type(value, expected) {
            return Err(StructuredOutputError::InvalidType {
                path: path.to_string(),
                expected: expected.to_string(),
                actual,
            });
        }
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object)
        && let Value::Object(map) = value
    {
        for (name, property_schema) in properties {
            if let Some(property_value) = map.get(name) {
                validate_node(property_value, property_schema, &format!("{path}.{name}"))?;
            }
        }
    }
    if let Some(required) = schema.get("required").and_then(Value::as_array)
        && let Value::Object(map) = value
    {
        for entry in required {
            let name = entry
                .as_str()
                .ok_or_else(|| StructuredOutputError::SchemaViolation {
                    path: path.to_string(),
                    message: "required entry is not a string".to_string(),
                })?;
            if !map.contains_key(name) {
                return Err(StructuredOutputError::MissingRequired {
                    path: path.to_string(),
                    property: name.to_string(),
                });
            }
        }
    }
    if schema.get("additionalProperties") == Some(&Value::Bool(false))
        && let Value::Object(map) = value
    {
        let allowed: Vec<&str> = schema
            .get("properties")
            .and_then(Value::as_object)
            .map(|properties| properties.keys().map(String::as_str).collect())
            .unwrap_or_default();
        for name in map.keys() {
            if !allowed.contains(&name.as_str()) {
                return Err(StructuredOutputError::DisallowedProperty {
                    path: path.to_string(),
                    property: name.clone(),
                });
            }
        }
    }
    if let Some(items) = schema.get("items")
        && let Value::Array(array) = value
    {
        for (index, item) in array.iter().enumerate() {
            validate_node(item, items, &format!("{path}[{index}]"))?;
        }
    }
    Ok(())
}

/// Whether a value satisfies the schema `type` keyword.
fn matches_type(value: &Value, expected: &str) -> bool {
    expected == "null" && value.is_null() || workflow_value_matches_type(value, expected)
}

/// A short label of a value's JSON type, used in mismatch errors.
fn type_label(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(_) => "boolean".to_string(),
        Value::Number(number) if number.is_i64() || number.is_u64() => "integer".to_string(),
        Value::Number(_) => "number".to_string(),
        Value::String(_) => "string".to_string(),
        Value::Array(_) => "array".to_string(),
        Value::Object(_) => "object".to_string(),
    }
}

/// Collects the bodies of every markdown fence (``` ... ``` or ~~~ ... ~~~) in document order.
///
/// Each fence body is tried as JSON, so a non-JSON fence is skipped naturally and the first fence
/// that parses as a JSON object wins, regardless of its language tag.
fn fenced_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some(marker) = fence_marker(line) else {
            continue;
        };
        let mut block = String::new();
        for inner in lines.by_ref() {
            if is_closing_fence(inner, marker) {
                break;
            }
            block.push_str(inner);
            block.push('\n');
        }
        blocks.push(block);
    }
    blocks
}

/// The fence marker of a line that opens a fence (``` or ~~~), if it opens one.
fn fence_marker(line: &str) -> Option<char> {
    let trimmed = line.trim_start();
    let marker = trimmed.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let count = trimmed.chars().take_while(|&c| c == marker).count();
    (count >= 3).then_some(marker)
}

/// Whether a line closes a fence opened with the given marker.
fn is_closing_fence(line: &str, marker: char) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with(marker) && trimmed.chars().take_while(|&c| c == marker).count() >= 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// A plain JSON object is extracted directly.
    #[test]
    fn extracts_a_bare_json_object() {
        let value = extract_json_object(r#"{ "approved": true, "score": 92 }"#).unwrap();
        assert_eq!(value, json!({ "approved": true, "score": 92 }));
    }

    /// A JSON object inside a single markdown fence is extracted.
    #[test]
    fn extracts_json_from_a_markdown_fence() {
        let text = "审查结果如下：\n\n```json\n{\"approved\": true, \"score\": 92}\n```\n";
        assert_eq!(
            extract_json_object(text).unwrap(),
            json!({ "approved": true, "score": 92 })
        );
    }

    /// A fence with a non-JSON language tag is skipped; a bare fence is accepted.
    #[test]
    fn skips_non_json_fences_and_accepts_bare_fences() {
        let text = "```rust\nlet x = 1;\n```\n```\n{\"approved\": true}\n```";
        assert_eq!(
            extract_json_object(text).unwrap(),
            json!({ "approved": true })
        );
    }

    /// Non-object results and missing fences fail with a clear reason.
    #[test]
    fn rejects_non_object_and_missing_json() {
        assert!(matches!(
            extract_json_object("[1, 2, 3]"),
            Err(StructuredOutputError::NotJsonObject { .. })
        ));
        assert!(matches!(
            extract_json_object("no json here"),
            Err(StructuredOutputError::NotJsonObject { .. })
        ));
    }

    /// A JSON object surrounded by prose but not fenced is still recovered.
    #[test]
    fn extracts_an_inline_json_object_surrounded_by_prose() {
        let text = "审查完成，结果为 {\"approved\": true, \"score\": 92}，请查收。";
        assert_eq!(
            extract_json_object(text).unwrap(),
            json!({ "approved": true, "score": 92 })
        );
    }

    /// A bare object followed by commentary is recovered from its leading balanced object.
    #[test]
    fn extracts_a_bare_object_with_trailing_commentary() {
        let text = "{\"approved\": true} 以上是本次结果。";
        assert_eq!(
            extract_json_object(text).unwrap(),
            json!({ "approved": true })
        );
    }

    /// A fenced object that also carries surrounding prose still parses from the embedded object.
    #[test]
    fn extracts_an_object_inside_a_prose_fence() {
        let text = "```json\n结果：{\"approved\": true}\n```";
        assert_eq!(
            extract_json_object(text).unwrap(),
            json!({ "approved": true })
        );
    }

    fn schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "approved": { "type": "boolean" },
                "score": { "type": "number" },
                "tags": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["approved"],
            "additionalProperties": false
        })
    }

    /// A conforming object validates, including nested array items.
    #[test]
    fn validates_a_conforming_object() {
        let value = json!({ "approved": true, "score": 92.5, "tags": ["a", "b"] });
        validate_against_schema(&value, &schema()).unwrap();
    }

    /// A missing required property is reported with its path and name.
    #[test]
    fn reports_a_missing_required_property() {
        let value = json!({ "score": 92 });
        assert_eq!(
            validate_against_schema(&value, &schema()).unwrap_err(),
            StructuredOutputError::MissingRequired {
                path: "$".to_string(),
                property: "approved".to_string(),
            }
        );
    }

    /// A type mismatch is reported at the offending path.
    #[test]
    fn reports_a_nested_type_mismatch() {
        let value = json!({ "approved": true, "score": "high" });
        assert_eq!(
            validate_against_schema(&value, &schema()).unwrap_err(),
            StructuredOutputError::InvalidType {
                path: "$.score".to_string(),
                expected: "number".to_string(),
                actual: "string".to_string(),
            }
        );
    }

    /// `additionalProperties: false` rejects undeclared keys.
    #[test]
    fn rejects_disallowed_properties() {
        let value = json!({ "approved": true, "extra": 1 });
        assert_eq!(
            validate_against_schema(&value, &schema()).unwrap_err(),
            StructuredOutputError::DisallowedProperty {
                path: "$".to_string(),
                property: "extra".to_string(),
            }
        );
    }

    /// `integer` rejects floats while `number` accepts them.
    #[test]
    fn distinguishes_integer_from_number() {
        assert!(validate_against_schema(&json!(5), &json!({ "type": "integer" })).is_ok());
        assert_eq!(
            validate_against_schema(&json!(5.5), &json!({ "type": "integer" })).unwrap_err(),
            StructuredOutputError::InvalidType {
                path: "$".to_string(),
                expected: "integer".to_string(),
                actual: "number".to_string(),
            }
        );
        assert!(validate_against_schema(&json!(5.5), &json!({ "type": "number" })).is_ok());
    }

    /// A missing `required` entry type or a bad array item is a schema violation.
    #[test]
    fn reports_schema_violations_for_bad_array_items() {
        let array_schema = json!({ "type": "array", "items": { "type": "boolean" } });
        assert_eq!(
            validate_against_schema(&json!([true, "x"]), &array_schema).unwrap_err(),
            StructuredOutputError::InvalidType {
                path: "$[1]".to_string(),
                expected: "boolean".to_string(),
                actual: "string".to_string(),
            }
        );
    }

    /// File fields accept only canonical safe Workspace-relative references.
    #[test]
    fn validates_file_and_file_array_fields() {
        let schema = json!({
            "type": "object",
            "properties": {
                "primary": { "type": "file" },
                "attachments": { "type": "array", "items": { "type": "file" } }
            },
            "required": ["primary", "attachments"]
        });
        assert!(
            validate_against_schema(
                &json!({
                    "primary": { "kind": "workspace_file", "path": "report.md" },
                    "attachments": [
                        { "kind": "workspace_file", "path": "assets/chart.png" }
                    ]
                }),
                &schema,
            )
            .is_ok()
        );
        assert!(matches!(
            validate_against_schema(
                &json!({ "primary": "report.md", "attachments": [] }),
                &schema,
            ),
            Err(StructuredOutputError::InvalidType { .. })
        ));
    }

    /// Invalid schema keywords fail before an Agent value can be accepted.
    #[test]
    fn rejects_invalid_schema_definitions() {
        assert!(matches!(
            validate_structured_output_schema(&json!({
                "type": "object",
                "properties": { "created": { "type": "date" } }
            })),
            Err(StructuredOutputError::SchemaViolation { .. })
        ));
        assert!(matches!(
            validate_structured_output_schema(&json!({
                "type": "object",
                "properties": {},
                "required": ["missing"]
            })),
            Err(StructuredOutputError::SchemaViolation { .. })
        ));
    }
}
