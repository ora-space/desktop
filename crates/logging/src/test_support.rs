use tracing_subscriber::Registry;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::Layer;
use tracing_subscriber::prelude::*;

use crate::{LogLevel, LogLevelControl};

/// Initializes the clock prerequisite for integration tests that compose production adapters.
///
/// Subscriber installation remains test-scoped. Repeated callers retain the first configured
/// timezone, just as production does; deterministic timestamp tests should inject their own clock.
pub fn initialize_test_clock() {
    let _ = crate::clock::initialize(chrono_tz::Asia::Shanghai);
}

/// Creates an isolated reload control for higher-level state and adapter tests.
#[doc(hidden)]
pub fn test_log_level_control(level: LogLevel) -> LogLevelControl {
    LogLevelControl::scoped_for_tests(level)
}

/// Runs one test body under a TRACE-enabled subscriber so shared callsites stay observable.
pub fn with_trace_logging<R>(action: impl FnOnce() -> R) -> R {
    let subscriber = tracing_subscriber::registry().with(LevelFilter::TRACE);

    tracing::subscriber::with_default(subscriber, action)
}

/// Runs one test body under a TRACE-enabled subscriber that records structured logging events.
pub fn with_recorded_trace_logging<L, R>(layer: L, action: impl FnOnce() -> R) -> R
where
    L: Layer<Registry> + Send + Sync + 'static,
{
    let subscriber = tracing_subscriber::registry()
        .with(layer)
        .with(LevelFilter::TRACE);

    tracing::subscriber::with_default(subscriber, action)
}
