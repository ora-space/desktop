//! Bounded best-effort redaction of Host-known runtime values from one plugin's stderr records.
//!
//! The Host holds the registry of values in `ora-plugin-lifecycle`; this module owns only the
//! immutable compiled matcher and the field-level substitution, so desktop and Node share one
//! matching implementation (ADR D1/D3).

use aho_corasick::AhoCorasick;
use serde_json::{Map, Value};

use crate::plugin_log::record::{
    DEFAULT_PLUGIN_TARGET, PluginLogRecord, RAW_STDERR_TARGET, RESERVED_CONTEXT_KEYS,
};

/// Minimum UTF-8 byte length of a value worth redacting (ADR D3): shorter values are themselves
/// unreliable as secrets and redacting them would hurt diagnostics more than it helps.
pub const MIN_REDACTION_BYTES: usize = 8;

/// Hard bounds on the registry (ADR D2): count and total UTF-8 bytes of registered values.
pub const MAX_REDACTION_VALUES: usize = 256;
pub const MAX_REDACTION_BYTES: usize = 64 * 1024;

/// The fixed placeholder every redacted span becomes (ADR D3), shared with `SessionMcpBinding`.
pub const REDACTION_PLACEHOLDER: &str = "[REDACTED]";

/// An immutable, compiled set of values one process generation redacts from plugin fields.
///
/// Empty by default; matching an empty matcher is a no-op, so a generation without registered
/// values runs the pipeline byte-for-byte as before the feature existed (invariant 2).
#[derive(Debug, Default, Clone)]
pub struct Redactions {
    matcher: Option<AhoCorasick>,
}

impl Redactions {
    /// Compiles already-filtered values into a matcher. Build failures are reported to the caller
    /// so the registry can treat the values as not registered (ADR D3).
    pub fn build(
        values: impl IntoIterator<Item = String>,
    ) -> Result<Self, aho_corasick::BuildError> {
        let values: Vec<String> = values.into_iter().collect();
        if values.is_empty() {
            return Ok(Self { matcher: None });
        }
        let matcher = AhoCorasick::new(values)?;
        Ok(Self {
            matcher: Some(matcher),
        })
    }

    /// Whether nothing is registered, so substitution can be skipped entirely.
    pub fn is_empty(&self) -> bool {
        self.matcher.is_none()
    }

    /// Replaces every registered value in `text`, merging overlapping or adjacent spans into one
    /// placeholder each so no registered byte leaks between spans (ADR D3).
    fn redact_text(&self, text: &mut String) {
        let Some(matcher) = &self.matcher else {
            return;
        };
        let mut spans: Vec<(usize, usize)> = matcher
            .find_overlapping_iter(text.as_bytes())
            .map(|found| (found.start(), found.end()))
            .collect();
        // Overlapping matches do not arrive start-ordered, so sort before merging. Replace from
        // the end so earlier byte offsets stay valid as the string shrinks.
        spans.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
        for (start, end) in spans {
            match merged.last_mut() {
                Some((_, last_end)) if start <= *last_end => *last_end = (*last_end).max(end),
                _ => merged.push((start, end)),
            }
        }
        for (start, end) in merged.into_iter().rev() {
            text.replace_range(start..end, REDACTION_PLACEHOLDER);
        }
    }
}

/// Applies `redactions` to every plugin-controlled string of a decoded record, before it is
/// rendered to JSONL. Host-written fields (`timestamp`, `level`) and the reserved top-level
/// `context` keys stay byte-for-byte unchanged (ADR D1, invariant 8).
pub fn redact_record(record: &mut PluginLogRecord, redactions: &Redactions) {
    if redactions.is_empty() {
        return;
    }
    redactions.redact_text(&mut record.message);
    // Raw records (`plugin.stderr`) and envelopes without an explicit target (`plugin`) own their
    // target to the Host; only a plugin-supplied target is plugin-controlled (ADR D1).
    if record.target != DEFAULT_PLUGIN_TARGET && record.target != RAW_STDERR_TARGET {
        redactions.redact_text(&mut record.target);
    }
    if let Some(method) = &mut record.method {
        redactions.redact_text(method);
    }
    if let Some(error) = &mut record.error {
        for value in error.values_mut() {
            redact_values_only(value, redactions);
        }
    }
    redact_context(&mut record.context, redactions);
}

/// Recursively redacts string values without touching object keys, used for the `error` field where
/// ADR D1 does not prompt keys.
fn redact_values_only(value: &mut Value, redactions: &Redactions) {
    match value {
        Value::String(text) => redactions.redact_text(text),
        Value::Array(items) => {
            for item in items {
                redact_values_only(item, redactions);
            }
        }
        Value::Object(map) => {
            for nested in map.values_mut() {
                redact_values_only(nested, redactions);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// Recursively redacts `context` keys and values, exempting the top-level reserved keys, and
/// rebuilds the map so identical keys after substitution collapse deterministically (ADR D1).
fn redact_context(context: &mut Map<String, Value>, redactions: &Redactions) {
    let mut rebuilt = Map::new();
    for (mut key, mut value) in std::mem::take(context) {
        if RESERVED_CONTEXT_KEYS.contains(&key.as_str()) {
            rebuilt.insert(key, value);
            continue;
        }
        redactions.redact_text(&mut key);
        redact_context_value(&mut value, redactions);
        // `Map` is ordered by key; the first (lexicographically smaller) key after substitution
        // wins so the result never depends on the order the plugin wrote its fields.
        rebuilt.entry(key).or_insert(value);
    }
    *context = rebuilt;
}

/// Recursively redacts a `context` value: nested objects also get their keys and values redacted.
fn redact_context_value(value: &mut Value, redactions: &Redactions) {
    match value {
        Value::String(text) => redactions.redact_text(text),
        Value::Array(items) => {
            for item in items {
                redact_context_value(item, redactions);
            }
        }
        Value::Object(map) => redact_nested_object(map, redactions),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// Rebuilds a nested `context` object, redacting keys and values. Nested keys are plugin-controlled
/// even when they repeat a reserved name, so no reserved-key exemption applies here (ADR D1).
fn redact_nested_object(map: &mut Map<String, Value>, redactions: &Redactions) {
    let mut rebuilt = Map::new();
    for (mut key, mut value) in std::mem::take(map) {
        redactions.redact_text(&mut key);
        redact_context_value(&mut value, redactions);
        rebuilt.entry(key).or_insert(value);
    }
    *map = rebuilt;
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use time::macros::datetime;

    fn redactions(values: &[&str]) -> Redactions {
        Redactions::build(values.iter().map(|value| (*value).to_string())).expect("matcher")
    }

    /// A raw record for the shared host origin, decoding `message` as raw stderr bytes.
    fn record(message: &str) -> PluginLogRecord {
        use crate::plugin_log::record::RecordOrigin;
        let origin = RecordOrigin {
            plugin_id: "official/example".to_string(),
            host_session_id: "session-a".to_string(),
            generation: 3,
        };
        let frame = ora_utils::text::LineFrame::Line(message.as_bytes().to_vec());
        crate::plugin_log::record::decode_frame(
            frame,
            &origin,
            datetime!(2026-09-14 10:00:00 +08:00),
        )
        .record
    }

    /// Overlapping registered values must leave no residue (core case: `abcdWXYZ` + `WXYZefgh`
    /// in `abcdWXYZefgh`).
    #[test]
    fn overlapping_spans_leave_no_residue() {
        let redactions = redactions(&["abcdWXYZ", "WXYZefgh"]);
        let mut text = "prefix abcdWXYZefgh suffix".to_string();
        redactions.redact_text(&mut text);
        assert_eq!(text, "prefix [REDACTED] suffix");
    }

    /// Adjacent spans merge into one placeholder.
    #[test]
    fn adjacent_spans_merge() {
        let redactions = redactions(&["abcdefgh", "ijklmnop"]);
        let mut text = "abcdefghijklmnop".to_string();
        redactions.redact_text(&mut text);
        assert_eq!(text, "[REDACTED]");
    }

    /// Host fields, the raw target, and reserved context keys survive a hostile registered value
    /// untouched; nested same-named keys and ordinary context are redacted.
    #[test]
    fn host_fields_and_reserved_keys_keep_their_bytes() {
        let redactions = redactions(&["official/example"]);
        let mut record = record("boom official/example");
        record
            .context
            .insert("secret".to_string(), json!("official/example"));
        record.context.insert(
            "nested".to_string(),
            json!({"plugin_id": "official/example"}),
        );
        redact_record(&mut record, &redactions);
        assert_eq!(record.target, "plugin.stderr");
        assert_eq!(record.message, "boom [REDACTED]");
        assert_eq!(record.context["plugin_id"], json!("official/example"));
        assert_eq!(record.context["nested"]["plugin_id"], json!("[REDACTED]"));
        assert_eq!(record.context["secret"], json!("[REDACTED]"));
    }

    /// A plugin-supplied target is redacted; numbers and booleans are never matched.
    #[test]
    fn plugin_target_is_redacted_but_scalars_are_not() {
        let redactions = redactions(&["secret_value"]);
        let mut record = record("plain");
        record.target = "plugin.secret_value".to_string();
        record.method = Some("secret_value".to_string());
        record.context.insert("number".to_string(), json!(99999));
        record.context.insert("bool".to_string(), json!(true));
        redact_record(&mut record, &redactions);
        assert_eq!(record.target, "plugin.[REDACTED]");
        assert_eq!(record.method.as_deref(), Some("[REDACTED]"));
        assert_eq!(record.context["number"], json!(99999));
        assert_eq!(record.context["bool"], json!(true));
    }

    /// Two context keys that collapse into the same placeholder keep the lexicographically first.
    #[test]
    fn key_collapse_keeps_the_lexicographically_first_entry() {
        let redactions = redactions(&["12345678"]);
        let mut record = record("plain");
        // `abc12345678` redacts to `abc[REDACTED]`, colliding with the plugin's own literal key.
        record
            .context
            .insert("abc12345678".to_string(), json!("first"));
        record
            .context
            .insert("abc[REDACTED]".to_string(), json!("second"));
        redact_record(&mut record, &redactions);
        assert_eq!(record.context.get("abc[REDACTED]"), Some(&json!("first")));
        assert!(!record.context.contains_key("abc12345678"));
    }

    /// An empty matcher leaves a record untouched, including key ordering.
    #[test]
    fn empty_redactions_are_a_noop() {
        let redactions = Redactions::default();
        let mut record = record("anything");
        let before = record.clone();
        redact_record(&mut record, &redactions);
        assert_eq!(record, before);
    }
}
