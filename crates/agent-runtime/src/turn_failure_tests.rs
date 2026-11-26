//! Unit coverage of the failure builder: sanitizing and bounding the Agent's text (case 3),
//! extracting the `data` supplement (case 1), and shaping `acp_failure` / `no_progress_failure`.
//!
//! Evidence for specs/test-cases/node/protocol/turn-failure-reason.md cases 1, 3, 4, 5 and 6.
use crate::turn_failure::{acp_failure, no_progress_failure, sanitize_agent_text};
use ora_history::{TurnFailureAnswer, TurnFailureCode};
use pretty_assertions::assert_eq;
use serde_json::json;

/// Case 3: newlines, carriage returns, tabs and the `Zl`/`Zp` separators become spaces.
#[test]
fn sanitize_replaces_newlines_and_separators_with_spaces() {
    let (text, truncated) = sanitize_agent_text("a\nb\r\tc\u{2028}d\u{2029}e", &None).unwrap();
    assert_eq!((text.as_str(), truncated), ("a b  c d e", false));
}

/// Case 3: remaining `Cc` controls and every `Cf` format character are deleted.
#[test]
fn sanitize_deletes_format_and_control_characters() {
    let (text, truncated) = sanitize_agent_text(
        "a\u{200B}b\u{200D}c\u{FEFF}d\u{E0000}e\u{0001}f\u{0008}g",
        &None,
    )
    .unwrap();
    assert_eq!((text.as_str(), truncated), ("abcdefg", false));
}

/// Case 3: surrounding Unicode whitespace is trimmed, and text that cleans to nothing is omitted.
#[test]
fn sanitize_trims_whitespace_and_drops_empty_text() {
    let (text, truncated) = sanitize_agent_text("\u{3000}  ok \u{00A0}", &None).unwrap();
    assert_eq!((text.as_str(), truncated), ("ok", false));
    assert_eq!(sanitize_agent_text("\n\t\u{200B}\u{3000}", &None), None);
}

/// Case 3: text over 1024 bytes is cut at a code point boundary and stays valid UTF-8.
#[test]
fn sanitize_truncates_at_the_byte_limit_on_a_code_point_boundary() {
    // 1023 ASCII bytes plus a 3-byte character crosses the limit inside the character.
    let primary = format!("{}中", "a".repeat(1023));
    let (text, truncated) = sanitize_agent_text(&primary, &None).unwrap();
    assert!(truncated);
    assert_eq!(text, "a".repeat(1023));
    assert!(std::str::from_utf8(text.as_bytes()).is_ok());
}

/// Case 1: the `data` supplement is appended after `: ` and not repeated when it equals the text.
#[test]
fn data_supplement_is_appended_and_deduplicated() {
    let cases: &[(&str, Option<serde_json::Value>, &str)] = &[
        ("err", None, "err"),
        ("err", Some(json!("detail")), "err: detail"),
        ("err", Some(json!({"message": "detail"})), "err: detail"),
        ("err", Some(json!("err")), "err"),
        ("err", Some(json!(42)), "err"),
        ("err", Some(json!([1, 2])), "err"),
        ("err", Some(json!({"other": "x"})), "err"),
    ];
    for (primary, data, expected) in cases {
        let (text, _) = sanitize_agent_text(primary, &data.clone()).unwrap();
        assert_eq!(&text, expected);
    }
}

/// Case 1: an ACP error becomes `agent_error` with its code and sanitized text; a text-based
/// authentication classification becomes `agent_authentication_required`.
#[test]
fn acp_failure_carries_the_code_and_sanitized_text() {
    let failure = acp_failure(
        -32603,
        "upstream \n diagnostic",
        &Some(json!("detail")),
        /*auth*/ false,
    );
    assert_eq!(failure.code, TurnFailureCode::AgentError);
    assert_eq!(failure.acp_code, Some(-32603));
    assert_eq!(failure.message.as_deref(), Some("upstream   diagnostic: detail"));
    assert_eq!(failure.attempts, None);
    assert_eq!(failure.answer, None);

    let auth = acp_failure(-32000, "Authentication required", &None, /*auth*/ true);
    assert_eq!(auth.code, TurnFailureCode::AgentAuthenticationRequired);
    assert_eq!(auth.acp_code, Some(-32000));
}

/// Cases 4, 5 and 6: a watchdog failure records its attempts, the answer and an optional ACP error.
#[test]
fn no_progress_failure_records_attempts_answer_and_acp_error() {
    let failure = no_progress_failure(
        /*attempts*/ 4,
        TurnFailureAnswer::Cancelled,
        None,
        None,
        None,
    );
    assert_eq!(failure.code, TurnFailureCode::NoProgress);
    assert_eq!(failure.attempts, Some(4));
    assert_eq!(failure.answer, Some(TurnFailureAnswer::Cancelled));
    assert_eq!(failure.acp_code, None);

    let answered = no_progress_failure(
        /*attempts*/ 2,
        TurnFailureAnswer::Error,
        Some(-32800),
        Some("cancelled".into()),
        None,
    );
    assert_eq!(answered.attempts, Some(2));
    assert_eq!(answered.answer, Some(TurnFailureAnswer::Error));
    assert_eq!(answered.acp_code, Some(-32800));
    assert_eq!(answered.message.as_deref(), Some("cancelled"));
}
