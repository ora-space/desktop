use std::io::Write;

use tracing::Dispatch;
use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::layer;
use tracing_subscriber::prelude::*;

use crate::correlation::CorrelationLayer;
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
            let (stdout_writer, stdout_guard) = prepare_stdout_output(stdout_writer);
            let subscriber = tracing_subscriber::registry()
                .with(CorrelationLayer)
                .with(level_filter)
                .with(
                    layer()
                        .event_format(JsonEventFormatter::new(config.timezone))
                        .with_writer(stdout_writer)
                        .with_ansi(false),
                );

            Ok((
                Dispatch::new(subscriber),
                LoggingGuard::new(vec![stdout_guard]),
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
            let prepared_output = prepare_file_output(file_config)?;
            let (stdout_writer, stdout_guard) = prepare_stdout_output(stdout_writer);
            let subscriber = tracing_subscriber::registry()
                .with(CorrelationLayer)
                .with(level_filter)
                .with(
                    layer()
                        .event_format(JsonEventFormatter::new(config.timezone))
                        .with_writer(stdout_writer)
                        .with_ansi(false),
                )
                .with(
                    layer()
                        .event_format(JsonEventFormatter::new(config.timezone))
                        .with_writer(prepared_output.writer.clone())
                        .with_ansi(false),
                );

            Ok((
                Dispatch::new(subscriber),
                LoggingGuard::new(vec![stdout_guard, prepared_output.guard]),
            ))
        }
    }
}

/// Creates an explicitly lossy non-blocking writer so stdout backpressure cannot stall async workers.
fn prepare_stdout_output<W>(stdout_writer: W) -> (NonBlocking, WorkerGuard)
where
    W: Write + Send + 'static,
{
    tracing_appender::non_blocking::NonBlockingBuilder::default()
        .lossy(true)
        .finish(stdout_writer)
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
