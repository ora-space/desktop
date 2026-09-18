use ora_process_protocol::{
    GUARDIAN_WIRE_VERSION, GuardianAccess, GuardianChannel, GuardianReady, GuardianReadyRequest,
};
use std::io;

/// Authenticates original-instance readiness without taking over control or authorizing Runs.
pub struct GuardianProbe {
    access: GuardianAccess,
    expected_uid: u32,
    session: [u8; 16],
}

impl GuardianProbe {
    /// Uses explicit OS identity and persisted recovery material supplied by the trusted owner.
    pub fn new(access: GuardianAccess, expected_uid: u32) -> Self {
        Self {
            access,
            expected_uid,
            session: *uuid::Uuid::new_v4().as_bytes(),
        }
    }

    /// Queries one physical channel and verifies the original intent and correlation nonce.
    pub async fn ready(&self, channel: GuardianChannel) -> io::Result<GuardianReady> {
        let request = GuardianReadyRequest {
            version: GUARDIAN_WIRE_VERSION,
            intent: self.access.intent.clone(),
            channel,
            session: self.session,
        };
        let ready: GuardianReady =
            crate::transport::exchange(&self.access, self.expected_uid, channel, &request).await?;
        let expected = GuardianReady {
            version: GUARDIAN_WIRE_VERSION,
            intent: self.access.intent.clone(),
            channel,
            session: self.session,
        };
        if ready != expected {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guardian Ready identity mismatch",
            ));
        }
        Ok(ready)
    }
}
