use std::{io, path::PathBuf};

use ora_process_protocol::{HOST_WIRE_VERSION, HostOperation, HostReply, HostRequest};

/// A reconnectable trusted-local host client; neither handles nor transport errors own remote lifetime.
#[derive(Clone)]
pub struct ProcessHost {
    state_dir: PathBuf,
    expected_uid: u32,
}

impl ProcessHost {
    pub fn new(state_dir: PathBuf, expected_uid: u32) -> Self {
        Self {
            state_dir,
            expected_uid,
        }
    }

    /// Every exchange reconnects to the original host directory and verifies semantic correlation.
    /// A lost reply is queried with the original Run/Scope identity, never retried as new work.
    pub async fn execute(&self, operation: HostOperation) -> io::Result<HostReply> {
        let endpoint = self.state_dir.join(operation.socket_name());
        let request = HostRequest {
            version: HOST_WIRE_VERSION,
            operation,
        };
        let reply: HostReply =
            crate::transport::exchange_path(&endpoint, self.expected_uid, &request).await?;
        let correlated = match (&request.operation, &reply) {
            (_, HostReply::Rejected(_)) | (HostOperation::Inspect, HostReply::Ready(_)) => true,
            (HostOperation::Start { intent }, HostReply::Run(view)) => {
                intent.scope == view.scope && intent.run == view.run
            }
            (
                HostOperation::QueryRun { run } | HostOperation::Stop { run },
                HostReply::Run(view),
            ) => *run == view.run,
            (
                HostOperation::CreateScope { scope }
                | HostOperation::QueryScope { scope }
                | HostOperation::Close { scope },
                HostReply::Scope(view),
            ) => *scope == view.scope,
            (
                HostOperation::Output {
                    run,
                    stream,
                    offset,
                    max_bytes,
                },
                HostReply::Output {
                    run: actual,
                    stream: actual_stream,
                    offset: actual_offset,
                    output,
                },
            ) => {
                run == actual
                    && stream == actual_stream
                    && offset == actual_offset
                    && output.bytes.len() <= *max_bytes
            }
            _ => false,
        };
        if !correlated {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "host reply identity mismatch",
            ));
        }
        Ok(reply)
    }
}
