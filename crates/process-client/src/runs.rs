use std::io;

use ora_process_protocol::{
    GUARDIAN_WIRE_VERSION, GuardianAccess, GuardianHostSession, GuardianRunOperation,
    GuardianRunReply, GuardianRunRequest, GuardianRunResult,
};

/// Trusted-local Run operations share the bounded guardian transport; no database or spawner lives here.
pub struct GuardianRuns {
    access: GuardianAccess,
    expected_uid: u32,
    session: GuardianHostSession,
}

impl GuardianRuns {
    pub fn new(access: GuardianAccess, expected_uid: u32, session: GuardianHostSession) -> Self {
        Self {
            access,
            expected_uid,
            session,
        }
    }

    /// Correlates the original attempt and operation after transport success; errors never imply non-acceptance.
    pub async fn execute(&self, operation: GuardianRunOperation) -> io::Result<GuardianRunResult> {
        let request = GuardianRunRequest {
            version: GUARDIAN_WIRE_VERSION,
            intent: self.access.intent.clone(),
            channel: operation.channel(),
            session: self.session.clone(),
            operation,
        };
        let reply: GuardianRunReply =
            crate::transport::exchange(&self.access, self.expected_uid, request.channel, &request)
                .await?;
        let correlated = match (&request.operation, &reply.result) {
            (_, GuardianRunResult::Rejected(_)) => true,
            (
                GuardianRunOperation::Start { run, .. }
                | GuardianRunOperation::Query { run }
                | GuardianRunOperation::Stop { run },
                GuardianRunResult::Run(snapshot),
            ) => *run == snapshot.id,
            (
                GuardianRunOperation::Close | GuardianRunOperation::Scope,
                GuardianRunResult::Scope(_),
            ) => true,
            (
                GuardianRunOperation::Output {
                    run,
                    stream,
                    offset,
                    max_bytes,
                },
                GuardianRunResult::Output {
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
        if reply.intent != self.access.intent || reply.session != self.session || !correlated {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guardian Run reply mismatch",
            ));
        }
        Ok(reply.result)
    }
}
