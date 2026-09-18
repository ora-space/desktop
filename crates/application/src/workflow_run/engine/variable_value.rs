use ora_utils::path::PortableRelativePath;
use serde_json::{Value, json};

/// Returns whether a wire type belongs to the workflow variable vocabulary.
pub(super) fn is_supported_variable_type(value_type: &str) -> bool {
    matches!(
        value_type,
        "string"
            | "number"
            | "integer"
            | "boolean"
            | "secret"
            | "file"
            | "object"
            | "any"
            | "array"
            | "array[string]"
            | "array[number]"
            | "array[object]"
            | "array[boolean]"
            | "array[file]"
            | "array[any]"
    )
}

/// Normalizes legacy file-path strings and rejects values outside their declared type.
///
/// File variables use an explicit object on the durable wire. Accepting strings only at this
/// boundary migrates existing graphs without letting the ambiguous representation spread further.
pub(super) fn normalize_workflow_value(value: Value, value_type: &str) -> Option<Value> {
    match value_type {
        "file" => normalize_file_reference(value),
        "array[file]" => value
            .as_array()?
            .iter()
            .cloned()
            .map(normalize_file_reference)
            .collect::<Option<Vec<_>>>()
            .map(Value::Array),
        _ if workflow_value_matches_type(&value, value_type) => Some(value),
        _ => None,
    }
}

/// Checks a JSON value against the workflow variable type vocabulary.
pub(super) fn workflow_value_matches_type(value: &Value, value_type: &str) -> bool {
    match value_type {
        "string" | "secret" => value.is_string(),
        "file" => is_file_reference(value),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "object" => value.is_object(),
        "any" => true,
        "array" | "array[any]" => value.is_array(),
        typed if typed.starts_with("array[") && typed.ends_with(']') => {
            value.as_array().is_some_and(|items| {
                let item_type = &typed[6..typed.len() - 1];
                items
                    .iter()
                    .all(|item| workflow_value_matches_type(item, item_type))
            })
        }
        _ => false,
    }
}

/// Converts a legacy path or canonical object into a safe Workspace-relative file reference.
fn normalize_file_reference(value: Value) -> Option<Value> {
    let raw_path = match value {
        Value::String(path) => path,
        Value::Object(reference) => {
            if reference.get("kind").and_then(Value::as_str) != Some("workspace_file") {
                return None;
            }
            reference.get("path")?.as_str()?.to_string()
        }
        _ => return None,
    };
    let path = PortableRelativePath::parse(&raw_path).ok()?;
    if path.is_root() {
        return None;
    }
    Some(json!({ "kind": "workspace_file", "path": path.as_str() }))
}

/// Returns whether a value is already a canonical safe Workspace-relative file reference.
fn is_file_reference(value: &Value) -> bool {
    file_reference_rejection(value).is_none() && value.is_object()
}

/// Explains why a JSON value is not a canonical workspace file reference.
///
/// `None` means either the value is not an object (the caller should keep a generic type error)
/// or it is a valid file reference. Structured-output validation uses the `Some` reasons so a
/// near-miss object such as `notes/nul.md` names the actual rejection instead of "expected file".
pub(super) fn file_reference_rejection(value: &Value) -> Option<String> {
    let Value::Object(reference) = value else {
        return None;
    };
    if reference.len() != 2 || !reference.contains_key("kind") || !reference.contains_key("path") {
        return Some("file reference must have exactly the keys \"kind\" and \"path\"".to_string());
    }
    if reference.get("kind").and_then(Value::as_str) != Some("workspace_file") {
        return Some("file reference kind must be \"workspace_file\"".to_string());
    }
    let Some(path) = reference.get("path").and_then(Value::as_str) else {
        return Some("file reference path must be a string".to_string());
    };
    match PortableRelativePath::parse(path) {
        Err(error) => Some(format!("file path {path:?} rejected: {error}")),
        Ok(parsed) if parsed.is_root() => Some("file path must not be empty".to_string()),
        Ok(parsed) if parsed.as_str() != path => Some(format!(
            "file path {path:?} must be written in canonical form {:?}",
            parsed.as_str()
        )),
        Ok(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn validates_every_supported_value_type() {
        let cases = [
            ("string", json!("text")),
            ("number", json!(1.5)),
            ("integer", json!(1)),
            ("boolean", json!(true)),
            ("secret", json!("hidden")),
            (
                "file",
                json!({ "kind": "workspace_file", "path": "docs/input.txt" }),
            ),
            ("object", json!({ "key": "value" })),
            ("any", Value::Null),
            ("array", json!([1, "two", false])),
            ("array[string]", json!(["one", "two"])),
            ("array[number]", json!([1, 2.5])),
            ("array[object]", json!([{ "one": 1 }, { "two": 2 }])),
            ("array[boolean]", json!([true, false])),
            (
                "array[file]",
                json!([{ "kind": "workspace_file", "path": "one.txt" }]),
            ),
            ("array[any]", json!([1, "two", null])),
        ];

        for (value_type, value) in cases {
            assert!(
                workflow_value_matches_type(&value, value_type),
                "{value_type} should accept {value}"
            );
        }
    }

    #[test]
    fn distinguishes_untyped_and_typed_arrays() {
        let mixed = json!([1, "two", { "three": 3 }]);
        assert!(workflow_value_matches_type(&mixed, "array"));
        assert!(workflow_value_matches_type(&mixed, "array[any]"));
        assert!(!workflow_value_matches_type(&mixed, "array[string]"));
        assert!(!workflow_value_matches_type(&mixed, "array[object]"));
    }

    #[test]
    fn migrates_legacy_file_paths_and_rejects_unsafe_references() {
        assert_eq!(
            normalize_workflow_value(json!("docs\\input.txt"), "file"),
            Some(json!({ "kind": "workspace_file", "path": "docs/input.txt" }))
        );
        assert_eq!(
            normalize_workflow_value(json!(["one.txt", "nested/two.txt"]), "array[file]"),
            Some(json!([
                { "kind": "workspace_file", "path": "one.txt" },
                { "kind": "workspace_file", "path": "nested/two.txt" }
            ]))
        );
        assert_eq!(normalize_workflow_value(json!("../secret"), "file"), None);
        assert_eq!(
            normalize_workflow_value(
                json!({ "kind": "workspace_file", "path": "C:\\secret" }),
                "file"
            ),
            None
        );
    }

    #[test]
    fn file_reference_rejection_covers_each_object_branch() {
        assert_eq!(file_reference_rejection(&json!("notes/a.md")), None);
        assert_eq!(
            file_reference_rejection(&json!({ "kind": "workspace_file" })),
            Some("file reference must have exactly the keys \"kind\" and \"path\"".to_string())
        );
        assert_eq!(
            file_reference_rejection(&json!({
                "kind": "other",
                "path": "notes/a.md"
            })),
            Some("file reference kind must be \"workspace_file\"".to_string())
        );
        assert_eq!(
            file_reference_rejection(&json!({
                "kind": "workspace_file",
                "path": 1
            })),
            Some("file reference path must be a string".to_string())
        );
        let reserved = file_reference_rejection(&json!({
            "kind": "workspace_file",
            "path": "notes/nul.md"
        }))
        .expect("reserved device name should reject");
        assert!(
            reserved.contains("Windows reserved device name"),
            "{reserved}"
        );
        let traversal = file_reference_rejection(&json!({
            "kind": "workspace_file",
            "path": "../x"
        }))
        .expect("parent traversal should reject");
        assert!(traversal.contains("parent traversal"), "{traversal}");
        assert_eq!(
            file_reference_rejection(&json!({
                "kind": "workspace_file",
                "path": ""
            })),
            Some("file path must not be empty".to_string())
        );
        assert_eq!(
            file_reference_rejection(&json!({
                "kind": "workspace_file",
                "path": "docs\\input.txt"
            })),
            Some(
                "file path \"docs\\\\input.txt\" must be written in canonical form \"docs/input.txt\""
                    .to_string()
            )
        );
        assert_eq!(
            file_reference_rejection(&json!({
                "kind": "workspace_file",
                "path": "docs/input.txt"
            })),
            None
        );
    }
}
