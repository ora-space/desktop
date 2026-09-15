//! Recognizes the versioned structured-log envelope the Plugin SDK writes to stderr.
//!
//! stderr is a mixed byte stream: SDK records, third-party library output, and native code all
//! share it. The envelope prefix only identifies the *format*; it grants nothing. Anything that
//! does not parse and validate as a v1 payload is handed back as a rejection so the caller can
//! preserve the bytes through the raw path instead of dropping them.

use ora_logging::LogLevel;
use serde_json::{Map, Value};

/// Prefix of every SDK structured record; the trailing space separates it from the JSON body.
pub const PLUGIN_LOG_ENVELOPE_V1_PREFIX: &str = "@ora/plugin-log/v1 ";

/// Prefix shared by every envelope version, used to notice a version the host does not speak.
const PLUGIN_LOG_ENVELOPE_FAMILY: &str = "@ora/plugin-log/";

/// Upper bound on `target` and `method` so a payload cannot smuggle a record-sized identifier.
pub const MAX_IDENTIFIER_BYTES: usize = 256;

/// The validated content of one v1 envelope; every field is still an untrusted plugin statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredPayload {
    pub level: LogLevel,
    pub message: String,
    pub target: Option<String>,
    pub method: Option<String>,
    pub context: Map<String, Value>,
    pub error: Option<Map<String, Value>>,
}

/// Why a logical record was not accepted as a v1 envelope.
///
/// `NotAnEnvelope` is the ordinary case (plain third-party output); the other variants are
/// bounded failure classes the host may count and name without copying the payload anywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeRejection {
    NotAnEnvelope,
    UnknownVersion,
    MalformedJson,
    InvalidPayload,
}

impl EnvelopeRejection {
    /// Names the failure class for the raw record's context and the host's bounded summary.
    pub fn format_failure(self) -> Option<&'static str> {
        match self {
            Self::NotAnEnvelope => None,
            Self::UnknownVersion => Some("unknown_version"),
            Self::MalformedJson => Some("malformed_json"),
            Self::InvalidPayload => Some("invalid_payload"),
        }
    }
}

/// Parses one logical stderr record as a v1 envelope or classifies why it is not one.
pub fn parse_envelope(record: &str) -> Result<StructuredPayload, EnvelopeRejection> {
    let Some(body) = record.strip_prefix(PLUGIN_LOG_ENVELOPE_V1_PREFIX) else {
        return Err(if record.starts_with(PLUGIN_LOG_ENVELOPE_FAMILY) {
            EnvelopeRejection::UnknownVersion
        } else {
            EnvelopeRejection::NotAnEnvelope
        });
    };
    let Value::Object(mut fields) =
        serde_json::from_str::<Value>(body).map_err(|_| EnvelopeRejection::MalformedJson)?
    else {
        return Err(EnvelopeRejection::InvalidPayload);
    };
    let level = match fields.remove("level") {
        Some(Value::String(level)) => parse_level(&level)?,
        Some(_) | None => return Err(EnvelopeRejection::InvalidPayload),
    };
    let message = match fields.remove("message") {
        Some(Value::String(message)) => message,
        Some(_) | None => return Err(EnvelopeRejection::InvalidPayload),
    };
    let target = optional_identifier(fields.remove("target"))?;
    let method = optional_identifier(fields.remove("method"))?;
    let context = match fields.remove("context") {
        Some(Value::Object(context)) => context,
        None | Some(Value::Null) => Map::new(),
        Some(_) => return Err(EnvelopeRejection::InvalidPayload),
    };
    let error = match fields.remove("error") {
        Some(Value::Object(error)) => Some(error),
        None | Some(Value::Null) => None,
        Some(_) => return Err(EnvelopeRejection::InvalidPayload),
    };
    Ok(StructuredPayload {
        level,
        message,
        target,
        method,
        context,
        error,
    })
}

/// Accepts only the exact uppercase level names the envelope contract defines.
fn parse_level(level: &str) -> Result<LogLevel, EnvelopeRejection> {
    match level {
        "TRACE" => Ok(LogLevel::Trace),
        "DEBUG" => Ok(LogLevel::Debug),
        "INFO" => Ok(LogLevel::Info),
        "WARN" => Ok(LogLevel::Warn),
        "ERROR" => Ok(LogLevel::Error),
        _ => Err(EnvelopeRejection::InvalidPayload),
    }
}

/// Validates an optional identifier field: absent or null is fine, anything else must be a
/// non-empty string within the identifier bound.
fn optional_identifier(value: Option<Value>) -> Result<Option<String>, EnvelopeRejection> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) if !text.is_empty() && text.len() <= MAX_IDENTIFIER_BYTES => {
            Ok(Some(text))
        }
        Some(_) => Err(EnvelopeRejection::InvalidPayload),
    }
}

#[cfg(test)]
mod tests {
    use super::{EnvelopeRejection, StructuredPayload, parse_envelope};
    use ora_logging::LogLevel;
    use pretty_assertions::assert_eq;
    use serde_json::{Map, json};

    /// A complete v1 payload round-trips every field with its declared type.
    #[test]
    fn parses_a_complete_v1_envelope() {
        let record = r#"@ora/plugin-log/v1 {"level":"WARN","message":"slow\nquery","target":"db","method":"query","context":{"ms":12},"error":{"name":"E"}}"#;
        let mut context = Map::new();
        context.insert("ms".to_string(), json!(12));
        let mut error = Map::new();
        error.insert("name".to_string(), json!("E"));
        assert_eq!(
            parse_envelope(record),
            Ok(StructuredPayload {
                level: LogLevel::Warn,
                message: "slow\nquery".to_string(),
                target: Some("db".to_string()),
                method: Some("query".to_string()),
                context,
                error: Some(error),
            })
        );
    }

    /// Each rejection class is distinguishable so the host can count them without the payload.
    #[test]
    fn classifies_every_rejection() {
        let long_target = "t".repeat(257);
        let cases = [
            ("plain text", EnvelopeRejection::NotAnEnvelope),
            ("[plugin:error] boom", EnvelopeRejection::NotAnEnvelope),
            (
                r#"{"level":"INFO","message":"json without prefix"}"#,
                EnvelopeRejection::NotAnEnvelope,
            ),
            (
                r#"@ora/plugin-log/v2 {"level":"INFO","message":"x"}"#,
                EnvelopeRejection::UnknownVersion,
            ),
            (
                "@ora/plugin-log/v1 {not json",
                EnvelopeRejection::MalformedJson,
            ),
            (
                "@ora/plugin-log/v1 [1,2]",
                EnvelopeRejection::InvalidPayload,
            ),
            (
                r#"@ora/plugin-log/v1 {"level":"info","message":"lowercase"}"#,
                EnvelopeRejection::InvalidPayload,
            ),
            (
                r#"@ora/plugin-log/v1 {"level":"INFO"}"#,
                EnvelopeRejection::InvalidPayload,
            ),
            (
                r#"@ora/plugin-log/v1 {"level":"INFO","message":1}"#,
                EnvelopeRejection::InvalidPayload,
            ),
            (
                r#"@ora/plugin-log/v1 {"level":"INFO","message":"x","context":[]}"#,
                EnvelopeRejection::InvalidPayload,
            ),
            (
                r#"@ora/plugin-log/v1 {"level":"INFO","message":"x","error":"boom"}"#,
                EnvelopeRejection::InvalidPayload,
            ),
            (
                r#"@ora/plugin-log/v1 {"level":"INFO","message":"x","target":""}"#,
                EnvelopeRejection::InvalidPayload,
            ),
        ];
        let long_target_record = format!(
            r#"@ora/plugin-log/v1 {{"level":"INFO","message":"x","target":"{long_target}"}}"#
        );
        let mut observed = cases
            .iter()
            .map(|(record, _)| parse_envelope(record).unwrap_err())
            .collect::<Vec<_>>();
        observed.push(parse_envelope(&long_target_record).unwrap_err());
        let mut expected = cases
            .iter()
            .map(|(_, rejection)| *rejection)
            .collect::<Vec<_>>();
        expected.push(EnvelopeRejection::InvalidPayload);
        assert_eq!(observed, expected);
    }

    /// Optional fields may be absent or null; unknown keys are ignored rather than rejected.
    #[test]
    fn tolerates_absent_optional_fields_and_unknown_keys() {
        assert_eq!(
            parse_envelope(
                r#"@ora/plugin-log/v1 {"level":"TRACE","message":"m","target":null,"context":null,"extra":true}"#
            ),
            Ok(StructuredPayload {
                level: LogLevel::Trace,
                message: "m".to_string(),
                target: None,
                method: None,
                context: Map::new(),
                error: None,
            })
        );
    }
}
