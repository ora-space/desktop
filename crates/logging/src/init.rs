use std::io::Write;

use tracing::Dispatch;
use tracing_appender::non_blocking::{ErrorCounter, NonBlocking, NonBlockingBuilder, WorkerGuard};
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
            // Match the file sink: move stdout writes off the calling thread so a slow pipe
            // cannot stall Tokio workers that emit tracing events.
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
                LoggingGuard::new(
                    vec![prepared_stdout.guard],
                    vec![prepared_stdout.drop_counter],
                ),
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
                LoggingGuard::new(
                    vec![prepared_output.guard],
                    vec![prepared_output.drop_counter],
                ),
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
                LoggingGuard::new(
                    vec![prepared_stdout.guard, prepared_output.guard],
                    vec![prepared_stdout.drop_counter, prepared_output.drop_counter],
                ),
            ))
        }
    }
}

/// Prepared stdout non-blocking writer state, including the drop counter callers must retain.
struct PreparedStdoutOutput {
    writer: NonBlocking,
    guard: WorkerGuard,
    drop_counter: ErrorCounter,
}

/// Creates an explicitly lossy non-blocking writer so stdout backpressure cannot stall async workers.
fn prepare_stdout_output<W>(stdout_writer: W) -> PreparedStdoutOutput
where
    W: Write + Send + 'static,
{
    // lossy(true) matches the file sink: prefer dropping under sustained backpressure over
    // blocking the caller, and keep drops observable through ErrorCounter.
    let (writer, guard) = NonBlockingBuilder::default()
        .lossy(/*is_lossy*/ true)
        .finish(stdout_writer);
    let drop_counter = writer.error_counter();

    PreparedStdoutOutput {
        writer,
        guard,
        drop_counter,
    }
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
