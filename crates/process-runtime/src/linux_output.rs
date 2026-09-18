use std::fs::File;
use std::os::fd::OwnedFd;
use std::process::Child;

use ora_process_protocol::{OutputRead, OutputState, OutputStream};
use ora_utils::process::{PipeCapture, PipeReadState};

use crate::PlatformError;

/// Output lifetime is separate from the process tracking record, including after cleanup.
pub(crate) struct CapturedOutput {
    stdout: Result<PipeCapture, String>,
    stderr: Result<PipeCapture, String>,
}

impl CapturedOutput {
    /// Retains setup failures after exec so the adapter can stop the original attempt, never retry it.
    pub(crate) fn start(child: &mut Child, stdout_limit: usize, stderr_limit: usize) -> Self {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "missing stdout pipe".to_owned())
            .and_then(|pipe| {
                PipeCapture::start(File::from(OwnedFd::from(pipe)), stdout_limit)
                    .map_err(|error| error.to_string())
            });
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "missing stderr pipe".to_owned())
            .and_then(|pipe| {
                PipeCapture::start(File::from(OwnedFd::from(pipe)), stderr_limit)
                    .map_err(|error| error.to_string())
            });
        Self { stdout, stderr }
    }

    /// Does not acquire output locks on the stop path.
    pub(crate) fn requires_stop(&self) -> bool {
        [&self.stdout, &self.stderr]
            .into_iter()
            .any(|capture| match capture {
                Ok(capture) => capture.failed(),
                Err(_) => true,
            })
    }

    /// Exposes volatile bytes and stream facts without implying durable or business completion.
    pub(crate) fn read(
        &self,
        stream: OutputStream,
        offset: usize,
        max_bytes: usize,
    ) -> Result<OutputRead, PlatformError> {
        let capture = match stream {
            OutputStream::Stdout => &self.stdout,
            OutputStream::Stderr => &self.stderr,
        };
        match capture {
            Ok(capture) => {
                let read = capture
                    .read(offset, max_bytes)
                    .map_err(|error| PlatformError(error.to_string()))?;
                Ok(OutputRead {
                    bytes: read.bytes,
                    retained: read.retained,
                    truncated: read.truncated,
                    state: match read.state {
                        PipeReadState::Open => OutputState::Open,
                        PipeReadState::Eof => OutputState::Eof,
                        PipeReadState::Failed(error) => OutputState::Failed(error),
                    },
                })
            }
            Err(error) => {
                if offset != 0 {
                    return Err(PlatformError("offset beyond retained output".into()));
                }
                Ok(OutputRead {
                    bytes: Vec::new(),
                    retained: 0,
                    truncated: false,
                    state: OutputState::Failed(error.clone()),
                })
            }
        }
    }
}
