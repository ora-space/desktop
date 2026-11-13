//! Shared fixtures for deterministic abnormal prompt scenarios.
//!
//! The fake agent journals each prompt it receives before it holds or dies. The journal entry
//! carries the ACP session id and the exact rendered prompt, so a test can wait for its unique
//! marker before dropping its own human stream or observing a process crash without racing the
//! initial send. The backend constructor here replaces only the inactivity windows, per test
//! instance, with a compressed schedule that still runs every retry/cancel/attribution path.

use super::open_ready_backend_with;
use crate::setup::DesktopTestSetup;
use ora_backend::Backend;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::time::Duration;

/// Prompt marker that makes the fake agent hold silently after recording prompt receipt.
pub(super) const SILENT_MARKER: &str = "[fake-agent-silent]";
/// Prompt marker that makes the fake plugin process exit after recording prompt receipt.
pub(super) const CRASH_MARKER: &str = "[fake-agent-crash]";
/// Prompt marker that keeps the plugin alive and silent until the stream owner drops its turn.
pub(super) const DROP_MARKER: &str = "[fake-agent-drop]";

/// Compressed production-shaped schedule: initial send plus three retries, all observed.
pub(super) const TEST_INACTIVITY_WINDOWS: &[Duration] = &[
    Duration::from_millis(500),
    Duration::from_millis(500),
    Duration::from_millis(500),
    Duration::from_millis(500),
];

/// Journal the fake agent writes after receiving each prompt, including its ACP session id.
const PROMPT_JOURNAL: &str = "acp_prompts.jsonl";

/// Opens a ready Backend whose prompt inactivity schedule is compressed on this instance only.
pub(super) fn open_abnormal_backend(
    setup: &DesktopTestSetup,
) -> Result<Backend, Box<dyn std::error::Error>> {
    open_ready_backend_with(setup, |paths| {
        Backend::open_with_test_prompt_inactivity_windows(paths, TEST_INACTIVITY_WINDOWS)
            .map_err(Into::into)
    })
}

/// Waits until the fake agent has journaled a prompt containing this test's unique marker.
///
/// Polling yields to the backend's runtime so the actor that owns the prompt can keep making
/// progress while the external plugin writes the journal. The marker identifies this request,
/// so a journal entry from an earlier prompt in the same test cannot satisfy the wait.
pub(super) async fn wait_for_prompt_marker(
    package_root: &Path,
    marker: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if prompt_marker_count(package_root, marker) > 0 {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            return Err(format!("the fake agent never received prompt marker {marker}").into());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Returns how many served prompts contained this test's unique marker.
pub(super) fn prompt_marker_count(package_root: &Path, marker: &str) -> usize {
    prompt_entries(package_root)
        .into_iter()
        .filter(|entry| !entry.session_id.is_empty() && entry.prompt.contains(marker))
        .count()
}

/// Reads one prompt journal entry, preserving the ACP session id and rendered prompt.
struct PromptEntry {
    session_id: String,
    prompt: String,
}

/// Reads the fake agent's prompt journal entries in served order.
fn prompt_entries(package_root: &Path) -> Vec<PromptEntry> {
    fs::read_to_string(package_root.join(PROMPT_JOURNAL))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let entry: Value = serde_json::from_str(line).ok()?;
            Some(PromptEntry {
                session_id: entry["sessionId"].as_str()?.to_string(),
                prompt: entry["prompt"].as_str()?.to_string(),
            })
        })
        .collect()
}
