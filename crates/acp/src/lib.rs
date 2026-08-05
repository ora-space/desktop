//! Minimal ACP v1 stdio peer used by Ora's provider-neutral agent runtime.

mod peer;
mod pending;

pub use peer::{
    AcpClient, AcpError, AcpInboundEvent, AcpPeer, PendingSessionRequest, PermissionRequest,
    SessionResponse,
};
