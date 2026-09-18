//! Turns one logical stderr record into the JSONL line the host persists.
//!
//! The persisted shape reuses the Ora runtime log envelope (`timestamp`, `level`, `target`,
//! `message`, optional `method`/`context`/`error`) so the same tooling can read both, but the
//! trusted fields are always written here by the host: the plugin may describe itself in
//! `context`, and it may even name `plugin_id`, yet what lands on disk is the identity the
//! process was launched under, the host session it ran in, and the generation it was.
//! Reserved keys the plugin supplies — identity, correlation, and the host's own decoding
//! markers — are removed before the host writes its own, so a record can never carry a forged
//! host fact.

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

/// Context keys only the host may write.
///
/// Identity and generation are stamped on every record; the correlation keys stay empty until a
/// host ↔ plugin correlation contract exists; the last three are the decoding markers the raw
/// path adds. A plugin supplying any of them is stripped, never trusted.
pub const RESERVED_CONTEXT_KEYS: [&str; 9] = [
    "plugin_id",
    "generation",
    "host_session_id",
    "request_id",
    "trace_id",
    "span",
    "encoding",
    "fragment",
    "format_failure",
];

/// Identity the host binds to every record of one process generation.
///
/// `host_session_id` is minted once per host start and never reused, and `generation` counts
/// launches of this plugin within that session; together they tell two records apart even when
/// a restarted host counts generations from one again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordOrigin {
    pub plugin_id: String,
    pub host_session_id: String,
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
                for key in RESERVED_CONTEXT_KEYS {
                    context.remove(key);
                }
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
    context.insert(
        "host_session_id".to_string(),
        Value::String(origin.host_session_id.clone()),
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
    use serde_json::{Map, Value, json};
    use time::macros::datetime;

    fn origin() -> RecordOrigin {
        RecordOrigin {
            plugin_id: "official/example".to_string(),
            host_session_id: "session-a".to_string(),
            generation: 3,
        }
    }

    /// The host identity every record of `origin()` carries.
    fn host_context() -> Map<String, Value> {
        let mut context = Map::new();
        context.insert("plugin_id".to_string(), json!("official/example"));
        context.insert("host_session_id".to_string(), json!("session-a"));
        context.insert("generation".to_string(), json!(3));
        context
    }

    const NOW: time::OffsetDateTime = datetime!(2026-09-14 10:00:00 +08:00);

    /// Host identity overwrites whatever the payload claimed; forged correlation keys and
    /// decoding markers are stripped from `context` rather than kept or promoted; ordinary
    /// context survives as the plugin's own statement.
    #[test]
    fn structured_records_carry_host_identity_and_never_promote_reserved_keys() {
        let frame = LineFrame::Line(
            br#"@ora/plugin-log/v1 {"level":"ERROR","message":"boom","context":{"plugin_id":"evil/other","generation":99,"host_session_id":"forged","request_id":"r1","trace_id":"t1","span":"s","encoding":"utf8","fragment":{"sequence":0},"format_failure":"none","user":"kept"},"error":{"name":"E"}}"#
                .to_vec(),
        );
        let decoded = decode_frame(frame, &origin(), NOW);
        let mut context = host_context();
        context.insert("user".to_string(), json!("kept"));
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

    /// Two hosts (or one host restarted) that both count generations from the same number still
    /// produce distinguishable records, because the session id differs.
    #[test]
    fn host_sessions_keep_equal_generations_apart() {
        let frame = || LineFrame::Line(b"same bytes".to_vec());
        let first = decode_frame(frame(), &origin(), NOW).record;
        let restarted = RecordOrigin {
            host_session_id: "session-b".to_string(),
            ..origin()
        };
        let second = decode_frame(frame(), &restarted, NOW).record;
        assert_eq!(
            (
                first.context["generation"].clone(),
                second.context["generation"].clone(),
                first.context["host_session_id"].clone(),
                second.context["host_session_id"].clone(),
                first == second,
            ),
            (
                json!(3),
                json!(3),
                json!("session-a"),
                json!("session-b"),
                false
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
            let mut context = host_context();
            if let Some(failure) = failure {
                context.insert("format_failure".to_string(), json!(failure));
            }
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
        let mut context = host_context();
        context.insert("encoding".to_string(), json!("escaped-bytes"));
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
