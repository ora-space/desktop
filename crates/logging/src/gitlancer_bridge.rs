use std::path::Path;

use gitlancer::logging::GitlancerLogger;

/// Forwards gitlancer git command events to tracing via `ora-logging` structured macros.
pub struct OraGitlancerLogger;

impl GitlancerLogger for OraGitlancerLogger {
    fn log_command(&self, _cwd: &Path, command: &str) {
        let details = git_command_details(command);
        crate::ora_info!(
            message = "git command",
            operation = details.operation,
            action = details.action,
            scope = details.scope,
            key = details.key,
        );
    }

    fn log_result(&self, duration_ms: u64, success: bool, exit_code: Option<i32>) {
        if success {
            crate::ora_info!(
                message = "git command completed",
                duration_ms,
                exit_code = ?exit_code,
            );
        } else {
            crate::ora_error!(
                message = "git command failed",
                duration_ms,
                exit_code = ?exit_code,
            );
        }
    }
}

/// Contains the bounded, non-sensitive fields that explain a Git command's intent.
struct GitCommandDetails<'a> {
    operation: &'a str,
    action: Option<&'a str>,
    scope: Option<&'a str>,
    key: Option<&'a str>,
}

/// Extracts stable Git intent while excluding user-controlled arguments, paths, and values.
fn git_command_details(command: &str) -> GitCommandDetails<'_> {
    let arguments = command.split_whitespace().skip(1).collect::<Vec<_>>();
    let operation = arguments
        .first()
        .copied()
        .filter(|operation| {
            operation
                .chars()
                .all(|character| character.is_ascii_alphabetic())
        })
        .unwrap_or("unknown");

    if operation != "config" {
        return GitCommandDetails {
            operation,
            action: None,
            scope: None,
            key: None,
        };
    }

    GitCommandDetails {
        operation,
        action: arguments
            .iter()
            .copied()
            .find(|argument| matches!(*argument, "--get" | "--list" | "--unset" | "--add")),
        scope: arguments
            .iter()
            .copied()
            .find(|argument| matches!(*argument, "--global" | "--local" | "--system")),
        key: arguments.iter().copied().find(|key| {
            key.starts_with("user.")
                && key.chars().all(|character| {
                    character.is_ascii_alphanumeric() || character == '.' || character == '-'
                })
        }),
    }
}

/// Registers `OraGitlancerLogger` as the process-wide gitlancer logger.
///
/// Only the first call takes effect; see `gitlancer::logging::register`.
pub fn register_gitlancer_logger() {
    gitlancer::logging::register(OraGitlancerLogger);
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::sync::{Arc, Mutex};

    use gitlancer::logging::GitlancerLogger;
    use pretty_assertions::assert_eq;
    use serde_json::Value;
    use tracing::dispatcher::with_default;

    use crate::build_dispatch;
    use crate::{LogLevel, LogOutput, LoggingConfig};

    use super::OraGitlancerLogger;

    #[test]
    fn log_command_emits_safe_config_intent() {
        let buffer = SharedBuffer::default();
        let (dispatch, guard) = build_dispatch(
            &LoggingConfig::new(LogLevel::Info, LogOutput::Stdout, chrono_tz::UTC),
            buffer.make_writer(),
        )
        .unwrap();

        with_default(&dispatch, || {
            OraGitlancerLogger.log_command(
                std::path::Path::new("/repo/project"),
                "git config --global --get user.name",
            );
        });
        drop(dispatch);
        drop(guard);

        let events = buffer.json_lines();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["level"], Value::String("INFO".to_string()));
        assert_eq!(
            events[0]["message"],
            Value::String("git command".to_string())
        );
        assert_eq!(
            events[0]["context"]["operation"],
            Value::String("config".to_string())
        );
        assert_eq!(
            events[0]["context"]["action"],
            Value::String("--get".to_string())
        );
        assert_eq!(
            events[0]["context"]["scope"],
            Value::String("--global".to_string())
        );
        assert_eq!(
            events[0]["context"]["key"],
            Value::String("user.name".to_string())
        );
    }

    #[test]
    fn log_result_emits_info_on_success() {
        let buffer = SharedBuffer::default();
        let (dispatch, guard) = build_dispatch(
            &LoggingConfig::new(LogLevel::Info, LogOutput::Stdout, chrono_tz::UTC),
            buffer.make_writer(),
        )
        .unwrap();

        with_default(&dispatch, || {
            OraGitlancerLogger.log_result(42, true, Some(0));
        });
        drop(dispatch);
        drop(guard);

        let events = buffer.json_lines();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["level"], Value::String("INFO".to_string()));
        assert_eq!(
            events[0]["message"],
            Value::String("git command completed".to_string())
        );
    }

    #[test]
    fn log_result_emits_error_on_failure() {
        let buffer = SharedBuffer::default();
        let (dispatch, guard) = build_dispatch(
            &LoggingConfig::new(LogLevel::Error, LogOutput::Stdout, chrono_tz::UTC),
            buffer.make_writer(),
        )
        .unwrap();

        with_default(&dispatch, || {
            OraGitlancerLogger.log_result(0, false, Some(1));
        });
        drop(dispatch);
        drop(guard);

        let events = buffer.json_lines();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["level"], Value::String("ERROR".to_string()));
        assert_eq!(
            events[0]["message"],
            Value::String("git command failed".to_string())
        );
    }

    #[derive(Clone, Debug, Default)]
    struct SharedBuffer {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl SharedBuffer {
        fn make_writer(&self) -> SharedBufferWriter {
            SharedBufferWriter {
                bytes: self.bytes.clone(),
            }
        }

        fn json_lines(&self) -> Vec<Value> {
            let contents = String::from_utf8(self.bytes.lock().unwrap().clone()).unwrap();
            contents
                .lines()
                .map(serde_json::from_str)
                .collect::<Result<Vec<_>, _>>()
                .unwrap_or_default()
        }
    }

    #[derive(Clone, Debug)]
    struct SharedBufferWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl<'w> tracing_subscriber::fmt::MakeWriter<'w> for SharedBufferWriter {
        type Writer = SharedBufferHandle;

        fn make_writer(&'w self) -> Self::Writer {
            SharedBufferHandle {
                bytes: self.bytes.clone(),
            }
        }
    }

    /// Appends formatted log bytes directly into the shared buffer for non-blocking sink tests.
    impl Write for SharedBufferWriter {
        /// Appends one formatted chunk to the shared in-memory capture buffer.
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        /// Completes the in-memory write contract without additional synchronization.
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct SharedBufferHandle {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl Write for SharedBufferHandle {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.bytes.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
