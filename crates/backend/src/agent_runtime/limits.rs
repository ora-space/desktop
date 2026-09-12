use std::time::Duration;

pub(super) const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(15);
pub(super) const SESSION_SETUP_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const CANCELLATION_GRACE: Duration = Duration::from_secs(5);
pub(super) const PROMPT_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub(super) const CONTRACT_QUEUE_CAPACITY: usize = 256;
pub(super) const MAX_PROMPT_BYTES: usize = 16 * 1024 * 1024;
