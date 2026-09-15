//! The Start node's typed input declarations: form controls, value types, and wire parsing.
//!
//! Start inputs keep their presentation control separate from their variable-pool type, so a
//! saved graph always declares the pool type while the editor picks the form control that
//! produces it. Legacy snapshots without an explicit control derive a compatible one.

use crate::workflow_run::engine::variable_value::{
    is_supported_variable_type, normalize_workflow_value,
};
use serde::Deserialize;
use std::collections::HashSet;

/// One typed variable declared by the Start node, optionally carrying its initial value.
#[derive(Debug, Clone, PartialEq)]
pub struct StartInputVariable {
    pub name: String,
    pub display_name: Option<String>,
    pub field_type: StartInputFieldType,
    pub value_type: String,
    pub required: bool,
    pub options: Vec<String>,
    pub max_length: Option<usize>,
    pub value: Option<serde_json::Value>,
}

/// Form control used to collect one Start variable without conflating UI and value types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartInputFieldType {
    TextInput,
    Paragraph,
    Select,
    Number,
    Checkbox,
    File,
    FileList,
    Json,
}

impl StartInputFieldType {
    /// Parses current field metadata or derives a compatible control for legacy snapshots.
    pub(crate) fn from_wire(field_type: Option<&str>, value_type: &str) -> Option<Self> {
        match field_type {
            Some("text-input") => Some(Self::TextInput),
            Some("paragraph") => Some(Self::Paragraph),
            Some("select") => Some(Self::Select),
            Some("number") => Some(Self::Number),
            Some("checkbox") => Some(Self::Checkbox),
            Some("file") => Some(Self::File),
            Some("file-list") => Some(Self::FileList),
            Some("json") => Some(Self::Json),
            Some(_) => None,
            None => match value_type {
                "number" | "integer" => Some(Self::Number),
                "boolean" => Some(Self::Checkbox),
                "file" => Some(Self::File),
                "array[file]" => Some(Self::FileList),
                "object" | "any" | "array" | "array[string]" | "array[number]"
                | "array[object]" | "array[boolean]" | "array[any]" => Some(Self::Json),
                "string" | "secret" => Some(Self::TextInput),
                _ => None,
            },
        }
    }

    /// Returns the exact variable-pool type emitted by current Start field controls.
    pub(crate) fn value_type(self) -> &'static str {
        match self {
            Self::TextInput | Self::Paragraph | Self::Select => "string",
            Self::Number => "number",
            Self::Checkbox => "boolean",
            Self::File => "file",
            Self::FileList => "array[file]",
            Self::Json => "object",
        }
    }
}

/// Wire shape of one typed Start input variable.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WireStartInputVariable {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub field_type: Option<String>,
    #[serde(default)]
    pub value_type: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub max_length: Option<usize>,
    #[serde(default)]
    pub value: Option<serde_json::Value>,
}

/// Validates Start declarations before they become variable-pool catalog entries.
pub(crate) fn into_start_input_variables(
    wire: Vec<WireStartInputVariable>,
) -> Result<Vec<StartInputVariable>, String> {
    let mut names = HashSet::new();
    let mut variables = Vec::with_capacity(wire.len());
    for variable in wire {
        let name = variable.name.unwrap_or_default().trim().to_string();
        if name.is_empty() || name.contains('.') {
            return Err("variable names must be non-empty and cannot contain dots".into());
        }
        if !names.insert(name.clone()) {
            return Err(format!("duplicate variable name {name}"));
        }
        let value_type = variable.value_type.unwrap_or_default();
        if !is_supported_variable_type(&value_type) {
            return Err(format!("variable {name} has unsupported type {value_type}"));
        }
        let field_type =
            StartInputFieldType::from_wire(variable.field_type.as_deref(), &value_type)
                .ok_or_else(|| format!("variable {name} has unsupported Start field type"))?;
        if variable.field_type.is_some() && field_type.value_type() != value_type {
            return Err(format!(
                "variable {name} field type does not produce declared type {value_type}"
            ));
        }
        let options = variable
            .options
            .into_iter()
            .map(|option| option.trim().to_string())
            .collect::<Vec<_>>();
        if field_type == StartInputFieldType::Select
            && (options.is_empty()
                || options.iter().any(String::is_empty)
                || options.iter().collect::<HashSet<_>>().len() != options.len())
        {
            return Err(format!(
                "variable {name} select options must be non-empty and unique"
            ));
        }
        if field_type != StartInputFieldType::Select && !options.is_empty() {
            return Err(format!(
                "variable {name} options are only supported for select fields"
            ));
        }
        let display_name = variable
            .display_name
            .map(|display_name| display_name.trim().to_string())
            .filter(|display_name| !display_name.is_empty());
        let max_length = match variable.max_length {
            Some(0) => return Err(format!("variable {name} max length must be positive")),
            Some(_)
                if !matches!(
                    field_type,
                    StartInputFieldType::TextInput | StartInputFieldType::Paragraph
                ) =>
            {
                return Err(format!(
                    "variable {name} max length is only supported for text fields"
                ));
            }
            max_length => max_length,
        };
        let value = match variable.value {
            Some(value) => Some(normalize_workflow_value(value, &value_type).ok_or_else(|| {
                format!("variable {name} value does not match declared type {value_type}")
            })?),
            None => None,
        };
        if let (Some(max_length), Some(serde_json::Value::String(value))) = (max_length, &value)
            && value.chars().count() > max_length
        {
            return Err(format!(
                "variable {name} value exceeds maximum length {max_length}"
            ));
        }
        if field_type == StartInputFieldType::Select
            && value
                .as_ref()
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| !options.iter().any(|option| option == value))
        {
            return Err(format!(
                "variable {name} initial value is not one of its select options"
            ));
        }
        variables.push(StartInputVariable {
            name,
            display_name,
            field_type,
            value_type,
            required: variable.required,
            options,
            max_length,
            value,
        });
    }
    Ok(variables)
}
