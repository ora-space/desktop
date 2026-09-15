//! Turns one logical stderr record into the JSONL line the host persists.
//!
//! The persisted shape reuses the Ora runtime log envelope (`timestamp`, `level`, `target`,
//! `message`, optional `method`/`context`/`error`) so the same tooling can read both, but the
//! trusted fields are always written here by the host: the plugin may describe itself in
//! `context`, and it may even name `plugin_id`, yet what lands on disk is the identity the
//! process was launched under.

use ora_logging::LogLevel;
use ora_utils::text::{ByteRendering, LineFrame, render_bytes_lossless};
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::plugin_log::envelope::{EnvelopeRejection, parse_envelope};

/// Target of every structured record whose payload named none.
pub const DEFAULT_PLUGIN_TARGET: &str = "plugin";

/// Target of every raw record: unstructured bytes the plugin or its dependencies wrote.
pub const RAW_STDERR_TARGET: &str = "plugin.stderr";

/// Identity the host binds to every record of one process generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordOrigin {
    pub plugin_id: String,
    pub generation: u64,
}

/// One record ready to be filtered and persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginLogRecord {
    pub timestamp: OffsetDateTime,
    pub level: LogLevel,
    pub target: String,
    pub message: String,
    pub method: Option<String>,
    pub context: Map<String, Value>,
    pub error: Option<Map<String, Value>>,
}

/// The decoded record plus the format-failure class the host may count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedRecord {
    pub record: PluginLogRecord,
    pub format_failure: Option<&'static str>,
}

impl PluginLogRecord {
    /// Renders the record as one JSON line, newline included.
    pub fn to_json_line(&self) -> String {
        let mut payload = Map::new();
        // RFC 3339 formatting of an offset datetime cannot fail; an empty string would only
        // ever appear if `time` changed that contract, and it still keeps the line well-formed.
        let timestamp = self.timestamp.format(&Rfc3339).unwrap_or_default();
        payload.insert("timestamp".to_string(), Value::String(timestamp));
        payload.insert(
            "level".to_string(),
            Value::String(self.level.as_upper_str().to_string()),
        );
        payload.insert("target".to_string(), Value::String(self.target.clone()));
        payload.insert("message".to_string(), Value::String(self.message.clone()));
        if let Some(method) = &self.method {
            payload.insert("method".to_string(), Value::String(method.clone()));
        }
        payload.insert("context".to_string(), Value::Object(self.context.clone()));
        if let Some(error) = &self.error {
            payload.insert("error".to_string(), Value::Object(error.clone()));
        }
        let mut line = Value::Object(payload).to_string();
        line.push('\n');
        line
    }
}

/// Decodes one frame into a record, taking the structured path when the frame is a valid v1
/// envelope and the raw path otherwise.
///
/// `now` is injected so the pipeline stamps records at receipt with the host clock while unit
/// tests stay deterministic.
pub fn decode_frame(frame: LineFrame, origin: &RecordOrigin, now: OffsetDateTime) -> DecodedRecord {
    match frame {
        LineFrame::Line(bytes) => decode_line(&strip_carriage_return(bytes), origin, now),
        LineFrame::Fragment {
            sequence,
            index,
            last,
            bytes,
        } => {
            let bytes = if last {
                strip_carriage_return(bytes)
            } else {
                bytes
            };
            let mut record = raw_record(&bytes, origin, now);
            record.context.insert(
                "fragment".to_string(),
                json!({ "sequence": sequence, "index": index, "last": last }),
            );
            DecodedRecord {
                record,
                format_failure: None,
            }
        }
    }
}

/// Decodes one complete logical record.
fn decode_line(bytes: &[u8], origin: &RecordOrigin, now: OffsetDateTime) -> DecodedRecord {
    // An envelope is UTF-8 by contract, so bytes that are not valid UTF-8 can only be raw.
    let rejection = match std::str::from_utf8(bytes) {
        Ok(text) => match parse_envelope(text) {
            Ok(payload) => {
                let mut context = payload.context;
                stamp_origin(&mut context, origin);
                return DecodedRecord {
                    record: PluginLogRecord {
                        timestamp: now,
                        level: payload.level,
                        target: payload
                            .target
                            .unwrap_or_else(|| DEFAULT_PLUGIN_TARGET.to_string()),
                        message: payload.message,
                        method: payload.method,
                        context,
                        error: payload.error,
                    },
                    format_failure: None,
                };
            }
            Err(rejection) => rejection,
        },
        Err(_) => EnvelopeRejection::NotAnEnvelope,
    };
    let mut record = raw_record(bytes, origin, now);
    let format_failure = rejection.format_failure();
    if let Some(class) = format_failure {
        record.context.insert(
            "format_failure".to_string(),
            Value::String(class.to_string()),
        );
    }
    DecodedRecord {
        record,
        format_failure,
    }
}

/// Builds the raw fallback record, preserving invalid bytes reversibly and saying so.
fn raw_record(bytes: &[u8], origin: &RecordOrigin, now: OffsetDateTime) -> PluginLogRecord {
    let (message, rendering) = render_bytes_lossless(bytes);
    let mut context = Map::new();
    if rendering == ByteRendering::Escaped {
        context.insert(
            "encoding".to_string(),
            Value::String("escaped-bytes".to_string()),
        );
    }
    stamp_origin(&mut context, origin);
    PluginLogRecord {
        timestamp: now,
        level: LogLevel::Info,
        target: RAW_STDERR_TARGET.to_string(),
        message: message.into_owned(),
        method: None,
        context,
        error: None,
    }
}

/// Writes the host-known identity last so nothing the plugin supplied can survive under the
/// trusted keys.
fn stamp_origin(context: &mut Map<String, Value>, origin: &RecordOrigin) {
    context.insert(
        "plugin_id".to_string(),
        Value::String(origin.plugin_id.clone()),
    );
    context.insert("generation".to_string(), json!(origin.generation));
}

/// Drops one trailing `\r` so CRLF producers do not leave a stray control character behind.
fn strip_carriage_return(mut bytes: Vec<u8>) -> Vec<u8> {
    if bytes.last() == Some(&b'\r') {
        bytes.pop();
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::{DecodedRecord, PluginLogRecord, RecordOrigin, decode_frame};
    use ora_logging::LogLevel;
    use ora_utils::text::LineFrame;
    use pretty_assertions::assert_eq;
    use serde_json::{Map, json};
    use time::macros::datetime;

    fn origin() -> RecordOrigin {
        RecordOrigin {
            plugin_id: "official/example".to_string(),
            generation: 3,
        }
    }

    const NOW: time::OffsetDateTime = datetime!(2026-09-14 10:00:00 +08:00);

    /// Host identity overwrites whatever the payload claimed, and reserved keys stay in
    /// `context` instead of being promoted to trusted top-level fields.
    #[test]
    fn structured_records_carry_host_identity_and_never_promote_reserved_keys() {
        let frame = LineFrame::Line(
            br#"@ora/plugin-log/v1 {"level":"ERROR","message":"boom","context":{"plugin_id":"evil/other","generation":99,"request_id":"r1","trace_id":"t1","span":"s"},"error":{"name":"E"}}"#
                .to_vec(),
        );
        let decoded = decode_frame(frame, &origin(), NOW);
        let mut context = Map::new();
        context.insert("plugin_id".to_string(), json!("official/example"));
        context.insert("generation".to_string(), json!(3));
        context.insert("request_id".to_string(), json!("r1"));
        context.insert("trace_id".to_string(), json!("t1"));
        context.insert("span".to_string(), json!("s"));
        let mut error = Map::new();
        error.insert("name".to_string(), json!("E"));
        assert_eq!(
            decoded,
            DecodedRecord {
                record: PluginLogRecord {
                    timestamp: NOW,
                    level: LogLevel::Error,
                    target: "plugin".to_string(),
                    message: "boom".to_string(),
                    method: None,
                    context,
                    error: Some(error),
                },
                format_failure: None,
            }
        );
        let line = decoded.record.to_json_line();
        let parsed: serde_json::Value = serde_json::from_str(line.trim_end()).expect("json line");
        assert_eq!(
            (
                line.ends_with('\n'),
                parsed.get("request_id"),
                parsed.get("trace_id"),
                parsed.get("span"),
                parsed["timestamp"].as_str(),
                parsed["level"].as_str(),
            ),
            (
                true,
                None,
                None,
                None,
                Some("2026-09-14T10:00:00+08:00"),
                Some("ERROR"),
            )
        );
    }

    /// Legacy SDK prefixes, invalid envelopes, and CRLF text all become raw `INFO` records under
    /// `plugin.stderr`, with invalid envelopes naming their failure class.
    #[test]
    fn everything_else_becomes_a_raw_info_record() {
        let cases: [(&[u8], Option<&str>); 4] = [
            (b"[plugin:error] boom\r", None),
            (b"@ora/plugin-log/v9 {}", Some("unknown_version")),
            (b"@ora/plugin-log/v1 {oops", Some("malformed_json")),
            (
                b"@ora/plugin-log/v1 {\"level\":\"NOPE\",\"message\":\"m\"}",
                Some("invalid_payload"),
            ),
        ];
        let expected_messages = [
            "[plugin:error] boom",
            "@ora/plugin-log/v9 {}",
            "@ora/plugin-log/v1 {oops",
            "@ora/plugin-log/v1 {\"level\":\"NOPE\",\"message\":\"m\"}",
        ];
        for ((bytes, failure), message) in cases.iter().zip(expected_messages) {
            let decoded = decode_frame(LineFrame::Line(bytes.to_vec()), &origin(), NOW);
            let mut context = Map::new();
            if let Some(failure) = failure {
                context.insert("format_failure".to_string(), json!(failure));
            }
            context.insert("plugin_id".to_string(), json!("official/example"));
            context.insert("generation".to_string(), json!(3));
            assert_eq!(
                decoded,
                DecodedRecord {
                    record: PluginLogRecord {
                        timestamp: NOW,
                        level: LogLevel::Info,
                        target: "plugin.stderr".to_string(),
                        message: message.to_string(),
                        method: None,
                        context,
                        error: None,
                    },
                    format_failure: *failure,
                }
            );
        }
    }

    /// Fragments and invalid UTF-8 keep their bytes and say how they were rendered.
    #[test]
    fn fragments_and_invalid_bytes_are_preserved_traceably() {
        let decoded = decode_frame(
            LineFrame::Fragment {
                sequence: 7,
                index: 2,
                last: true,
                bytes: b"tail\xff\r".to_vec(),
            },
            &origin(),
            NOW,
        );
        let mut context = Map::new();
        context.insert("encoding".to_string(), json!("escaped-bytes"));
        context.insert("plugin_id".to_string(), json!("official/example"));
        context.insert("generation".to_string(), json!(3));
        context.insert(
            "fragment".to_string(),
            json!({ "sequence": 7, "index": 2, "last": true }),
        );
        assert_eq!(
            decoded,
            DecodedRecord {
                record: PluginLogRecord {
                    timestamp: NOW,
                    level: LogLevel::Info,
                    target: "plugin.stderr".to_string(),
                    message: "tail\\xff".to_string(),
                    method: None,
                    context,
                    error: None,
                },
                format_failure: None,
            }
        );
    }
}
