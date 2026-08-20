//! Cooperative cancellation shared by download backends.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A shareable cancellation signal that in-flight downloads poll between chunks.
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    cancelled: Arc<AtomicBool>,
}

impl CancelToken {
    /// Creates a token that is not yet cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation; a download observes it on its next progress checkpoint.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    /// Returns true once cancellation was requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}
