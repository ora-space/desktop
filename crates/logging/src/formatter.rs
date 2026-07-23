use serde_json::{Map, Value, json};
use time::format_description::well_known::Rfc3339;
use tracing::Event;
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::registry::LookupSpan;

use crate::clock::now_local_in;
use crate::correlation::scope_correlation;

/// Formats every tracing event as one JSON line that follows the shared runtime envelope.
#[derive(Clone, Copy, Debug)]
pub(crate) struct JsonEventFormatter {
    timezone: chrono_tz::Tz,
}

impl JsonEventFormatter {
    /// Creates a formatter whose timestamps use the injected process timezone.
    pub(crate) fn new(timezone: chrono_tz::Tz) -> Self {
        Self { timezone }
    }
}

impl<S, N> FormatEvent<S, N> for JsonEventFormatter
where
    S: tracing::Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    /// Serializes the current event, its structured fields, and its active correlation scope.
    fn format_event(
        &self,
        context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let mut visitor = EventFieldVisitor::default();
        event.record(&mut visitor);

        let scope = scope_correlation(context.event_scope());
        let mut payload = Map::new();
        let timestamp = now_local_in(self.timezone)
            .format(&Rfc3339)
            .map_err(|_| std::fmt::Error)?;
        payload.insert("timestamp".to_string(), json!(timestamp));
        payload.insert(
            "level".to_string(),
            Value::String(event.metadata().level().to_string()),
        );
        payload.insert(
            "target".to_string(),
            Value::String(event.metadata().target().to_string()),
        );
        payload.insert(
            "message".to_string(),
            Value::String(visitor.message.unwrap_or_default()),
        );
        if let Some(method) = visitor.method {
            payload.insert("method".to_string(), Value::String(method));
        }

        if let Some(span_name) = visitor.span.or(scope.name) {
            payload.insert("span".to_string(), Value::String(span_name));
        }
        if let Some(trace_id) = visitor.trace_id.or(scope.trace_id) {
            payload.insert("trace_id".to_string(), Value::String(trace_id));
        }
        if let Some(request_id) = visitor.request_id.or(scope.request_id) {
            payload.insert("request_id".to_string(), Value::String(request_id));
        }
        if !visitor.context.is_empty() {
            payload.insert("context".to_string(), Value::Object(visitor.context));
        }
        if !visitor.error.is_empty() {
            payload.insert("error".to_string(), Value::Object(visitor.error));
        }

        writeln!(writer, "{}", Value::Object(payload))
    }
}

/// Collects event fields while enforcing the shared top-level, context, and error conventions.
#[derive(Debug, Default)]
struct EventFieldVisitor {
    message: Option<String>,
    method: Option<String>,
    span: Option<String>,
    trace_id: Option<String>,
    request_id: Option<String>,
    context: Map<String, Value>,
    error: Map<String, Value>,
}

impl EventFieldVisitor {
    /// Routes one recorded field into the correct part of the JSON envelope.
    fn record_value(&mut self, field_name: &str, value: Value) {
        match field_name {
            "message" => {
                self.message = value
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| Some(value.to_string()));
            }
            "method" => {
                self.method = value
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| Some(value.to_string()));
            }
            "span" => {
                self.span = value
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| Some(value.to_string()));
            }
            "trace_id" => {
                self.trace_id = value
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| Some(value.to_string()));
            }
            "request_id" => {
                self.request_id = value
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| Some(value.to_string()));
            }
            _ if field_name.starts_with("context.") => {
                if let Some(key) = field_name.strip_prefix("context.") {
                    self.context.insert(key.to_string(), value);
                }
            }
            _ if field_name.starts_with("error.") => {
                if let Some(key) = field_name.strip_prefix("error.") {
                    self.error.insert(key.to_string(), value);
                }
            }
            _ => {
                self.context.insert(field_name.to_string(), value);
            }
        }
    }
}

impl tracing::field::Visit for EventFieldVisitor {
    /// Preserves string event fields without debug-format noise in the JSON output.
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.record_value(field.name(), Value::String(value.to_string()));
    }

    /// Preserves boolean event fields so downstream assertions can treat them as booleans.
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.record_value(field.name(), Value::Bool(value));
    }

    /// Preserves signed integer fields as JSON numbers for downstream consumers.
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.record_value(field.name(), json!(value));
    }

    /// Preserves unsigned integer fields as JSON numbers for downstream consumers.
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.record_value(field.name(), json!(value));
    }

    /// Preserves floating-point fields as JSON numbers when serialization succeeds.
    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.record_value(field.name(), json!(value));
    }

    /// Falls back to debug formatting for field types that do not expose a more structured recorder.
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.record_value(
            field.name(),
            Value::String(format!("{value:?}").trim_matches('"').to_string()),
        );
    }
}
