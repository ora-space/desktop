use ora_utils::hash::sha256_reader;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::io::Cursor;
use thiserror::Error;

/// Maximum accepted byte length of one immutable declaration file.
pub const MAX_DECLARATION_BYTES: usize = 256 * 1024;
/// Maximum number of Settings one version-one declaration may render.
pub const MAX_SETTINGS: usize = 128;

/// Holds one validated and deterministically ordered declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompiledDeclaration {
    pub schema_version: u32,
    pub settings: Vec<SettingDeclaration>,
    pub fingerprint: String,
}

/// Describes one Setting independently of its current Stored Setting Value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingDeclaration {
    pub id: String,
    pub title: String,
    pub description: String,
    pub setting_type: SettingType,
    pub required: bool,
    pub order: Option<i64>,
    pub default: Option<SettingValue>,
}

/// Enumerates the value types supported by declaration schema version one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingType {
    String,
    Number,
    Boolean,
}

/// Carries one JSON scalar supported by Plugin Configuration version one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SettingValue {
    String(String),
    Number(serde_json::Number),
    Boolean(bool),
}

/// Reports a declaration that cannot be interpreted without ambiguity.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CompileDeclarationError {
    #[error("configuration declaration exceeds the {MAX_DECLARATION_BYTES}-byte limit")]
    TooLarge,
    #[error("configuration declaration contains a duplicate JSON key")]
    DuplicateKey,
    #[error("configuration declaration is not valid JSON: {0}")]
    InvalidJson(String),
    #[error("configuration declaration does not match schema version one: {0}")]
    InvalidStructure(String),
    #[error("unsupported configuration schema version {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("configuration declaration must contain at least one Setting")]
    EmptySettings,
    #[error("configuration declaration exceeds the {MAX_SETTINGS}-Setting limit")]
    TooManySettings,
    #[error("invalid Setting `{setting_id}`: {reason}")]
    InvalidSetting { setting_id: String, reason: String },
    #[error("failed to fingerprint the compiled declaration: {0}")]
    Fingerprint(String),
}

/// Compiles one declaration payload after applying the version-one structural rules.
pub fn compile_declaration(source: &[u8]) -> Result<CompiledDeclaration, CompileDeclarationError> {
    if source.len() > MAX_DECLARATION_BYTES {
        return Err(CompileDeclarationError::TooLarge);
    }
    let value = parse_strict_json(source)?;
    let canonical = serde_json::to_vec(&value)
        .map_err(|error| CompileDeclarationError::Fingerprint(error.to_string()))?;
    let raw: RawDeclaration = serde_json::from_value(value)
        .map_err(|error| CompileDeclarationError::InvalidStructure(error.to_string()))?;
    if raw.schema_version != 1 {
        return Err(CompileDeclarationError::UnsupportedSchemaVersion(
            raw.schema_version,
        ));
    }
    if raw.settings.is_empty() {
        return Err(CompileDeclarationError::EmptySettings);
    }
    if raw.settings.len() > MAX_SETTINGS {
        return Err(CompileDeclarationError::TooManySettings);
    }

    let mut settings = raw
        .settings
        .into_iter()
        .map(|(id, setting)| compile_setting(id, setting))
        .collect::<Result<Vec<_>, _>>()?;
    settings.sort_by(|left, right| match (left.order, right.order) {
        (Some(left_order), Some(right_order)) => left_order
            .cmp(&right_order)
            .then_with(|| left.id.cmp(&right.id)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => left.id.cmp(&right.id),
    });
    let fingerprint = sha256_reader(Cursor::new(canonical))
        .map_err(|error| CompileDeclarationError::Fingerprint(error.to_string()))?;

    Ok(CompiledDeclaration {
        schema_version: 1,
        settings,
        fingerprint,
    })
}

/// Parses one bounded JSON payload while retaining the distinction between duplicate and invalid syntax.
pub(crate) fn parse_strict_json(source: &[u8]) -> Result<Value, CompileDeclarationError> {
    let mut deserializer = serde_json::Deserializer::from_slice(source);
    let value = NoDuplicateValue
        .deserialize(&mut deserializer)
        .map_err(|error| {
            if error.to_string().starts_with(DUPLICATE_KEY_MARKER) {
                CompileDeclarationError::DuplicateKey
            } else {
                CompileDeclarationError::InvalidJson(error.to_string())
            }
        })?;
    deserializer
        .end()
        .map_err(|error| CompileDeclarationError::InvalidJson(error.to_string()))?;
    Ok(value)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawDeclaration {
    schema_version: u32,
    settings: BTreeMap<String, RawSetting>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawSetting {
    #[serde(rename = "type")]
    setting_type: SettingType,
    title: String,
    description: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    order: Option<i64>,
    #[serde(default)]
    default: RawDefault,
}

#[derive(Default)]
enum RawDefault {
    #[default]
    Missing,
    Present(Value),
}

impl<'de> Deserialize<'de> for RawDefault {
    /// Preserves the distinction between an omitted default and an explicit JSON null value.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Value::deserialize(deserializer).map(Self::Present)
    }
}

/// Validates one keyed Setting and preserves non-blank author text verbatim.
fn compile_setting(
    id: String,
    raw: RawSetting,
) -> Result<SettingDeclaration, CompileDeclarationError> {
    if !valid_setting_id(&id) {
        return Err(invalid_setting(
            id,
            "ID must match ^[a-z][A-Za-z0-9]{0,63}$",
        ));
    }
    validate_text(&id, "title", &raw.title, 120)?;
    validate_text(&id, "description", &raw.description, 2_000)?;
    let default = match raw.default {
        RawDefault::Missing => None,
        RawDefault::Present(value) => Some(setting_value(&id, raw.setting_type, value)?),
    };

    Ok(SettingDeclaration {
        id,
        title: raw.title,
        description: raw.description,
        setting_type: raw.setting_type,
        required: raw.required,
        order: raw.order,
        default,
    })
}

/// Converts a JSON scalar only when it matches the Setting's declared type.
fn setting_value(
    setting_id: &str,
    setting_type: SettingType,
    value: Value,
) -> Result<SettingValue, CompileDeclarationError> {
    match (setting_type, value) {
        (SettingType::String, Value::String(value)) => Ok(SettingValue::String(value)),
        (SettingType::Number, Value::Number(value)) => Ok(SettingValue::Number(value)),
        (SettingType::Boolean, Value::Bool(value)) => Ok(SettingValue::Boolean(value)),
        (SettingType::String, _) => Err(invalid_setting(setting_id, "default must be a string")),
        (SettingType::Number, _) => Err(invalid_setting(
            setting_id,
            "default must be a finite number",
        )),
        (SettingType::Boolean, _) => Err(invalid_setting(setting_id, "default must be a boolean")),
    }
}

/// Applies the ASCII identifier grammar without pulling regular expressions into the public path.
fn valid_setting_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    matches!(bytes.first(), Some(first) if first.is_ascii_lowercase())
        && bytes.len() <= 64
        && bytes.iter().all(u8::is_ascii_alphanumeric)
}

/// Applies the plain-text non-blank and character-count limits shared by labels and descriptions.
fn validate_text(
    setting_id: &str,
    field: &str,
    value: &str,
    max_characters: usize,
) -> Result<(), CompileDeclarationError> {
    if value.trim().is_empty() {
        return Err(invalid_setting(
            setting_id,
            format!("{field} must not be blank"),
        ));
    }
    if value.chars().count() > max_characters {
        return Err(invalid_setting(
            setting_id,
            format!("{field} exceeds {max_characters} characters"),
        ));
    }
    Ok(())
}

/// Attaches the stable Setting ID to one declaration failure.
fn invalid_setting(
    setting_id: impl Into<String>,
    reason: impl Into<String>,
) -> CompileDeclarationError {
    CompileDeclarationError::InvalidSetting {
        setting_id: setting_id.into(),
        reason: reason.into(),
    }
}

const DUPLICATE_KEY_MARKER: &str = "duplicate JSON key";

/// A recursive JSON seed that refuses to collapse duplicate object members.
struct NoDuplicateValue;

impl<'de> DeserializeSeed<'de> for NoDuplicateValue {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicateVisitor)
    }
}

/// Reconstructs ordinary JSON values while tracking every object key at its own nesting level.
struct NoDuplicateVisitor;

impl<'de> Visitor<'de> for NoDuplicateVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("JSON number must be finite"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(NoDuplicateValue)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = BTreeMap::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom(format!("{DUPLICATE_KEY_MARKER} `{key}`")));
            }
            values.insert(key, map.next_value_seed(NoDuplicateValue)?);
        }
        Ok(Value::Object(values.into_iter().collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CompileDeclarationError, CompiledDeclaration, SettingDeclaration, SettingType,
        SettingValue, compile_declaration,
    };
    use pretty_assertions::assert_eq;

    /// Duplicate Setting IDs must never depend on a JSON parser's first/last-value behaviour.
    #[test]
    fn rejects_duplicate_setting_ids() {
        let source = br#"{
            "schemaVersion": 1,
            "settings": {
                "endpoint": {"type":"string","title":"Endpoint","description":"URL"},
                "endpoint": {"type":"boolean","title":"Enabled","description":"Flag"}
            }
        }"#;

        assert_eq!(
            compile_declaration(source),
            Err(CompileDeclarationError::DuplicateKey)
        );
    }

    /// Version-one declarations compile into deterministic order without relying on JSON order.
    #[test]
    fn compiles_supported_settings_in_declared_order() {
        let source = br#"{
            "schemaVersion": 1,
            "settings": {
                "enabled": {"type":"boolean","title":"Enabled","description":"Use it"},
                "retries": {"type":"number","title":"Retries","description":"Attempts","order":1,"default":3},
                "endpoint": {"type":"string","title":"Endpoint","description":"Service URL","order":1,"required":true}
            }
        }"#;

        assert_eq!(
            compile_declaration(source).unwrap(),
            CompiledDeclaration {
                schema_version: 1,
                settings: vec![
                    SettingDeclaration {
                        id: "endpoint".to_string(),
                        title: "Endpoint".to_string(),
                        description: "Service URL".to_string(),
                        setting_type: SettingType::String,
                        required: true,
                        order: Some(1),
                        default: None,
                    },
                    SettingDeclaration {
                        id: "retries".to_string(),
                        title: "Retries".to_string(),
                        description: "Attempts".to_string(),
                        setting_type: SettingType::Number,
                        required: false,
                        order: Some(1),
                        default: Some(SettingValue::Number(3.into())),
                    },
                    SettingDeclaration {
                        id: "enabled".to_string(),
                        title: "Enabled".to_string(),
                        description: "Use it".to_string(),
                        setting_type: SettingType::Boolean,
                        required: false,
                        order: None,
                        default: None,
                    },
                ],
                fingerprint: "2ccfa6a76cc78a933643b89f84fd21620ff54e3825efca4d525c61e0c6b5049e"
                    .to_string(),
            }
        );
    }
}
