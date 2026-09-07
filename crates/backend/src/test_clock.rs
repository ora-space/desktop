/// Keeps persistence fixtures independent of process-wide logging initialization.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TestClock;

impl ora_db::TimestampSource for TestClock {
    /// Supplies a stable audit sample; business-version timestamps remain fixture inputs.
    fn current_timestamp_millis(&self) -> i64 {
        1
    }
}
