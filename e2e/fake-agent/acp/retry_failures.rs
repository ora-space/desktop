//! Test-only prompt fault injection for the fake agent.
//!
//! These markers let workflow tests hold the first attempt, fail it, and make the retry exit
//! without relying on timing windows.

use super::AcpError;
use ora_plugin_protocol::INTERNAL_ERROR_CODE;
use std::fs::OpenOptions;
use std::io::Write;

/// Prompt marker `[fail-times:N]`: the first N prompts carrying it answer with a JSON-RPC error.
pub(super) const FAIL_TIMES_MARKER: &str = "[fail-times:";
/// Prompt marker `[fail-delay-ms:N]`: hold the prompt this long before the planned failure.
pub(super) const FAIL_DELAY_MARKER: &str = "[fail-delay-ms:";
/// Prompt marker that defers the first failure until a load follower has attached.
pub(super) const FAIL_AFTER_LOAD_MARKER: &str = "[fail-after-load]";
/// Handshake file written when the deferred first prompt is waiting for its load follower.
pub(super) const FIRST_PROMPT_WAITING: &str = "first_prompt_waiting_for_load";
/// Prompt marker that makes the retry exit after the first prompt already failed.
pub(super) const CRASH_ON_RETRY_MARKER: &str = "[crash-on-retry]";

/// Journal of prompts failed on purpose, so the count survives new sessions and a restart.
const PROMPT_FAILURE_JOURNAL: &str = "prompt_failures.txt";

/// Whether the first attempt with the stale-load marker has already failed.
pub(super) fn first_prompt_failed() -> bool {
    std::fs::read_to_string(PROMPT_FAILURE_JOURNAL)
        .is_ok_and(|journal| journal.contains("[fail-times:1]"))
}

/// Signals the test that the first prompt is waiting for its load follower.
pub(super) fn mark_first_prompt_waiting() {
    let _ = std::fs::write(FIRST_PROMPT_WAITING, b"waiting");
}

/// Records the deferred first failure so the retry no longer defers or fails.
pub(super) fn record_first_failure() {
    if let Ok(mut journal) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(PROMPT_FAILURE_JOURNAL)
    {
        let _ = writeln!(journal, "[fail-times:1]");
    }
}

/// Reads `[fail-delay-ms:N]` so a test can attach a load follower during the first attempt.
pub(super) fn prompt_delay(prompt: &str) -> Option<u64> {
    let start = prompt.find(FAIL_DELAY_MARKER)?;
    let length = prompt[start..].find(']')? + 1;
    prompt[start + FAIL_DELAY_MARKER.len()..start + length - 1]
        .parse()
        .ok()
}

/// Fails this prompt when it carries `[fail-times:N]` and fewer than N prompts with the same
/// marker have failed so far.
pub(super) fn planned_prompt_failure(prompt: &str) -> Option<AcpError> {
    let start = prompt.find(FAIL_TIMES_MARKER)?;
    let length = prompt[start..].find(']')? + 1;
    let marker = &prompt[start..start + length];
    let times: usize = marker[FAIL_TIMES_MARKER.len()..length - 1].parse().ok()?;
    let failed = std::fs::read_to_string(PROMPT_FAILURE_JOURNAL)
        .unwrap_or_default()
        .lines()
        .filter(|line| *line == marker)
        .count();
    if failed >= times {
        return None;
    }
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(PROMPT_FAILURE_JOURNAL)
    {
        let _ = writeln!(file, "{marker}");
    }
    Some(AcpError {
        code: INTERNAL_ERROR_CODE,
        message: format!(
            "fake agent failed prompt {} of {times} on purpose",
            failed + 1
        ),
    })
}
