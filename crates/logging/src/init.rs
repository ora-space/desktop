use std::io::Write;

use tracing::Dispatch;
use tracing_appender::non_blocking::{NonBlocking, NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::layer;
use tracing_subscriber::prelude::*;

use crate::correlation::CorrelationLayer;
use crate::fanout::FanoutMakeWriter;
use crate::file_output::prepare_file_output;
use crate::formatter::JsonEventFormatter;
use crate::{LogLevel, LogOutput, LoggingConfig, LoggingGuard, LoggingInitError};

/// Installs the configured process clock and subscriber, then returns its writer-lifetime guard.
pub fn init_logging(config: LoggingConfig) -> Result<LoggingGuard, LoggingInitError> {
    // Prepare fallible sinks before changing either irreversible process-wide singleton.
    let (dispatch, guard) = build_dispatch(&config, std::io::stdout())?;
    crate::clock::initialize(config.timezone)
        .map_err(|_| LoggingInitError::ClockAlreadyInitialized)?;
    tracing::dispatcher::set_global_default(dispatch)
        .map_err(LoggingInitError::SetGlobalSubscriber)?;

    Ok(guard)
}

/// Builds a reusable tracing dispatch so tests can exercise sink behavior without global mutation.
pub(crate) fn build_dispatch<W>(
    config: &LoggingConfig,
    stdout_writer: W,
) -> Result<(Dispatch, LoggingGuard), LoggingInitError>
where
    W: Write + Send + 'static,
{
    let level_filter = level_filter(config.level);

    match &config.output {
        LogOutput::Stdout => {
            // Move stdout writes off the calling thread so a slow pipe cannot routinely stall
            // Tokio workers that emit tracing events. Prefer backpressure over dropping when
            // the queue is full so log integrity wins under sustained sink stalls.
            let prepared_stdout = prepare_stdout_output(stdout_writer);
            let subscriber = tracing_subscriber::registry()
                .with(CorrelationLayer)
                .with(level_filter)
                .with(
                    layer()
                        .event_format(JsonEventFormatter::new(config.timezone))
                        .with_writer(prepared_stdout.writer)
                        .with_ansi(false),
                );

            Ok((
                Dispatch::new(subscriber),
                LoggingGuard::new(vec![prepared_stdout.guard]),
            ))
        }
        LogOutput::File(file_config) => {
            let prepared_output = prepare_file_output(file_config)?;
            let subscriber = tracing_subscriber::registry()
                .with(CorrelationLayer)
                .with(level_filter)
                .with(
                    layer()
                        .event_format(JsonEventFormatter::new(config.timezone))
                        .with_writer(prepared_output.writer.clone())
                        .with_ansi(false),
                );

            Ok((
                Dispatch::new(subscriber),
                LoggingGuard::new(vec![prepared_output.guard]),
            ))
        }
        LogOutput::StdoutAndFile(file_config) => {
            // Serialize each event once and fan the formatted bytes out to stdout and the file
            // sink, instead of stacking two fmt layers that each run a full serialization pass.
            let prepared_output = prepare_file_output(file_config)?;
            let prepared_stdout = prepare_stdout_output(stdout_writer);
            let fanout =
                FanoutMakeWriter::new(prepared_stdout.writer, prepared_output.writer.clone());
            let subscriber = tracing_subscriber::registry()
                .with(CorrelationLayer)
                .with(level_filter)
                .with(
                    layer()
                        .event_format(JsonEventFormatter::new(config.timezone))
                        .with_writer(fanout)
                        .with_ansi(false),
                );

            Ok((
                Dispatch::new(subscriber),
                LoggingGuard::new(vec![prepared_stdout.guard, prepared_output.guard]),
            ))
        }
    }
}

/// Prepared stdout non-blocking writer state that callers must retain via `LoggingGuard`.
struct PreparedStdoutOutput {
    writer: NonBlocking,
    guard: WorkerGuard,
}

/// Creates a non-lossy non-blocking writer so routine stdout IO stays off caller threads.
fn prepare_stdout_output<W>(stdout_writer: W) -> PreparedStdoutOutput
where
    W: Write + Send + 'static,
{
    // Prefer backpressure over dropping when the queue is full: Ora prioritizes log integrity
    // over never blocking under extreme sink stalls. Normal log volume stays far below capacity.
    let (writer, guard) = NonBlockingBuilder::default()
        .lossy(/*is_lossy*/ false)
        .finish(stdout_writer);

    PreparedStdoutOutput { writer, guard }
}

/// Maps the public level enum into the tracing filter used by every active sink.
fn level_filter(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Trace => LevelFilter::TRACE,
        LogLevel::Debug => LevelFilter::DEBUG,
        LogLevel::Info => LevelFilter::INFO,
        LogLevel::Warn => LevelFilter::WARN,
        LogLevel::Error => LevelFilter::ERROR,
    }
}
