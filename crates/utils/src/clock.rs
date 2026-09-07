//! Injectable wall-clock timestamps; ordering and domain policy belong to consumers.

/// Supplies Unix milliseconds without imposing a runtime or a timezone implementation.
///
/// Implementations may return an earlier value after a clock adjustment. Consumers must use
/// explicit versions for causality and preserve any required timestamp ordering themselves.
pub trait TimestampSource {
    /// Returns the current wall-clock instant in Unix milliseconds.
    fn current_timestamp_millis(&self) -> i64;
}

impl<T: TimestampSource + ?Sized> TimestampSource for &T {
    /// Borrows a clock so a single injected source can serve multiple adapters.
    fn current_timestamp_millis(&self) -> i64 {
        T::current_timestamp_millis(self)
    }
}
