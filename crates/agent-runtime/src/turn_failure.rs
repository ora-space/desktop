//! Building the bounded `TurnFailure` a turn records when it did not complete, from the facts the
//! runtime observed. The code and the Agent's sanitized text are recorded here; callers write them
//! onto `TurnEnded` through `end_turn_with` / `end_timed_turn_with`.

use ora_history::{TurnFailure, TurnFailureAnswer, TurnFailureCode};
use ora_logging::ora_warn;
use serde_json::Value;

/// How many UTF-8 bytes of the Agent's text a `TurnFailure.message` may hold (D3).
pub(crate) const MAX_FAILURE_MESSAGE_BYTES: usize = 1024;

/// Reports whether a character is in Unicode category `Cf` (format), including the bidirectional
/// controls, zero-width joins, U+FEFF and the tag characters. These can fake rendered order or hide
/// text, so they are deleted rather than shown beside Ora's own wording.
fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{0600}'..='\u{0605}'
            | '\u{061C}'
            | '\u{06DD}'
            | '\u{070F}'
            | '\u{0890}'..='\u{0891}'
            | '\u{08E2}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// The sanitized Agent text: `message` carries the bounded cleaned text, `truncated` says it was
/// cut at the byte limit. `None` text means it cleaned to nothing and must be omitted.
pub(crate) fn sanitize_agent_text(primary: &str, data: &Option<Value>) -> Option<(String, bool)> {
    let mut text = primary.to_string();
    if let Some(supplement) = data_supplement_text(data) {
        if !supplement.is_empty() && supplement != primary {
            text.push_str(": ");
            text.push_str(&supplement);
        }
    }
    let cleaned: String = text
        .chars()
        .filter_map(|c| {
            if matches!(c, '\n' | '\r' | '\t' | '\u{2028}' | '\u{2029}') {
                Some(' ')
            } else if c.is_control() || is_format_char(c) {
                None
            } else {
                Some(c)
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(char::is_whitespace);
    if cleaned.is_empty() {
        return None;
    }
    if cleaned.len() <= MAX_FAILURE_MESSAGE_BYTES {
        return Some((cleaned.to_owned(), false));
    }
    let mut end = 0;
    for (at, c) in cleaned.char_indices() {
        if at + c.len_utf8() > MAX_FAILURE_MESSAGE_BYTES {
            break;
        }
        end = at + c.len_utf8();
    }
    Some((cleaned[..end].to_owned(), true))
}

/// Extracts the supplemental text from an ACP error's `data`: a string, or a string `message` field
/// on an object. Other shapes carry no prose Ora can safely show.
fn data_supplement_text(data: &Option<Value>) -> Option<&str> {
    match data {
        Some(Value::String(text)) => Some(text),
        Some(Value::Object(map)) => match map.get("message") {
            Some(Value::String(text)) => Some(text),
            _ => None,
        },
        _ => None,
    }
}

/// Maps the Agent's ACP request error onto a `TurnFailure`. The `auth` flag is produced by the
/// runtime's authentication classification (code `-32000` or the two known texts), which keeps the
/// single source of truth for it in one place.
pub(crate) fn acp_failure(code: i64, message: &str, data: &Option<Value>, auth: bool) -> TurnFailure {
    let sanitized = sanitize_agent_text(message, data);
    TurnFailure {
        code: if auth {
            TurnFailureCode::AgentAuthenticationRequired
        } else {
            TurnFailureCode::AgentError
        },
        acp_code: Some(code),
        message: sanitized.as_ref().map(|(text, _)| text.clone()),
        message_truncated: sanitized.as_ref().and_then(|(_, truncated)| {
            if *truncated {
                Some(true)
            } else {
                None
            }
        }),
        attempts: None,
        answer: None,
    }
}

/// Builds a `no_progress` failure from the watchdog's final state.
pub(crate) fn no_progress_failure(
    attempts: u32,
    answer: TurnFailureAnswer,
    acp_code: Option<i64>,
    message: Option<String>,
    message_truncated: Option<bool>,
) -> TurnFailure {
    TurnFailure {
        code: TurnFailureCode::NoProgress,
        acp_code,
        message,
        message_truncated,
        attempts: Some(attempts),
        answer: Some(answer),
    }
}

/// Codes the runtime turns names in logs and telemetry, kept value-light so no Agent text leaks.
pub(crate) fn failure_code(c: &TurnFailureCode) -> &'static str {
    match c {
        TurnFailureCode::AgentAuthenticationRequired => "agent_authentication_required",
        TurnFailureCode::AgentError => "agent_error",
        TurnFailureCode::NoProgress => "no_progress",
        TurnFailureCode::AgentUnavailable => "agent_unavailable",
        TurnFailureCode::Internal => "internal",
    }
}

/// Logs only the bounded fields of a failure; the sanitized Agent text never reaches a log.
pub(crate) fn log_turn_failure(prefix: &str, failure: &TurnFailure) {
    match (&failure.acp_code, &failure.answer) {
        (Some(code), Some(answer)) => ora_warn!(
            prefix = prefix,
            code = failure_code(&failure.code),
            acp_code = code,
            answer = ?answer,
        ),
        (Some(code), None) => ora_warn!(
            prefix = prefix,
            code = failure_code(&failure.code),
            acp_code = code,
        ),
        (None, Some(answer)) => ora_warn!(
            prefix = prefix,
            code = failure_code(&failure.code),
            answer = ?answer,
        ),
        (None, None) => ora_warn!(prefix = prefix, code = failure_code(&failure.code)),
    }
}