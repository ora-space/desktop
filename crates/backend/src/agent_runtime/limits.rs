use std::time::Duration;

pub(super) const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(15);
pub(super) const SESSION_SETUP_TIMEOUT: Duration = Duration::from_secs(30);
pub(super) const CANCELLATION_GRACE: Duration = Duration::from_secs(5);
/// Meaningful-activity windows for one prompt: the first send, then each automatic
/// retry after the previous window expired without progress.
///
/// Every retry re-sends the prompt as a fresh LLM turn, so the windows widen rather
/// than repeat: a stall that outlived 45 s is more likely an upstream slowdown than a
/// glitch, and a wider window avoids cancelling a slow-but-alive retry while limiting
/// how many duplicate turns a wedged agent is asked for.
pub(super) const PROMPT_INACTIVITY_WINDOWS: [Duration; 4] = [
    Duration::from_secs(45),
    Duration::from_secs(60),
    Duration::from_secs(90),
    Duration::from_secs(120),
];
pub(super) const CONTRACT_QUEUE_CAPACITY: usize = 256;
pub(super) const MAX_PROMPT_BYTES: usize = 16 * 1024 * 1024;
