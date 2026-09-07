use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

/// Gives repository tests an independently controlled clock, including deterministic rollback.
#[derive(Clone, Debug)]
pub(crate) struct TestClock(Arc<AtomicI64>);

impl TestClock {
    pub(crate) fn new(now: i64) -> Self {
        Self(Arc::new(AtomicI64::new(now)))
    }

    /// Changes the instant observed by every clone at the next transaction boundary.
    pub(crate) fn set(&self, now: i64) {
        self.0.store(now, Ordering::SeqCst);
    }
}

impl crate::TimestampSource for TestClock {
    /// Reads the fixture's local clock without changing process environment or timezone.
    fn current_timestamp_millis(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
