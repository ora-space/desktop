//! Per-runtime knobs used by Desktop E2E tests.
//!
//! These are compiled only with the `test-support` feature. Production keeps the fixed
//! inactivity schedule in `limits.rs` and never reaches this module.

use crate::{AgentRuntimeHost, AgentRuntimeManager};
use std::time::Duration;

impl<H: AgentRuntimeHost> AgentRuntimeManager<H> {
    /// Replaces the inactivity windows for actors created after this call.
    ///
    /// The manager still drives the ordinary retry, cancellation, and attribution paths; the
    /// windows merely make the schedule observable within a test's deadline instead of waiting
    /// the production 45-120 seconds. Production callers use the `PROMPT_INACTIVITY_WINDOWS`
    /// constant and never construct this test-support manager.
    pub fn set_test_prompt_inactivity_windows(&self, windows: &'static [Duration]) {
        assert!(
            !windows.is_empty(),
            "a prompt inactivity schedule needs at least its initial window",
        );
        *self
            .inner
            .prompt_inactivity_windows
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(windows);
    }
}
