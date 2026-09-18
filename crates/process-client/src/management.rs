use ora_process_protocol::{
    GUARDIAN_WIRE_VERSION, GuardianAccess, GuardianChannel, GuardianHostSession,
    GuardianManagementOperation, GuardianManagementReply, GuardianManagementRequest, HostBinding,
};
use std::io;

/// Trusted host-only management, not Controller authorization or a workload capability.
pub struct GuardianManagement {
    access: GuardianAccess,
    expected_uid: u32,
}

impl GuardianManagement {
    pub fn new(access: GuardianAccess, expected_uid: u32) -> Self {
        Self {
            access,
            expected_uid,
        }
    }

    /// Binds the incarnation committed under host ownership; retrying the same binding is idempotent.
    /// The trusted caller must obtain this binding from HostState, not a Node-supplied epoch.
    pub async fn bind(&self, host: HostBinding) -> io::Result<GuardianManagementReply> {
        self.exchange(
            GuardianChannel::Control,
            GuardianManagementOperation::Bind { host },
        )
        .await
    }

    /// Checks one channel against current host authority; it neither renews a lease nor authorizes Runs.
    pub async fn inspect(
        &self,
        channel: GuardianChannel,
        session: GuardianHostSession,
    ) -> io::Result<GuardianManagementReply> {
        self.exchange(channel, GuardianManagementOperation::Inspect { session })
            .await
    }

    /// Verifies semantic reply correlation after the shared authenticated transport exchange.
    async fn exchange(
        &self,
        channel: GuardianChannel,
        operation: GuardianManagementOperation,
    ) -> io::Result<GuardianManagementReply> {
        let request = GuardianManagementRequest {
            version: GUARDIAN_WIRE_VERSION,
            intent: self.access.intent.clone(),
            channel,
            operation,
        };
        let reply: GuardianManagementReply =
            crate::transport::exchange(&self.access, self.expected_uid, channel, &request).await?;
        let matches = match (&request.operation, &reply) {
            (
                GuardianManagementOperation::Bind { host },
                GuardianManagementReply::Bound { intent, session },
            ) => intent == &self.access.intent && session.host == *host,
            (
                GuardianManagementOperation::Inspect { session },
                GuardianManagementReply::Current {
                    intent,
                    session: current,
                    channel: actual,
                },
            ) => intent == &self.access.intent && current == session && actual == &channel,
            (_, GuardianManagementReply::Rejected(_)) => true,
            (GuardianManagementOperation::Bind { .. }, GuardianManagementReply::Current { .. })
            | (
                GuardianManagementOperation::Inspect { .. },
                GuardianManagementReply::Bound { .. },
            ) => false,
        };
        if !matches {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guardian management reply mismatch",
            ));
        }
        Ok(reply)
    }
}
