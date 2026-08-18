//! Minimal ACP v1 peer used by Ora's provider-neutral agent runtime.
//!
//! The peer is transport-neutral: it consumes and produces whole JSON-RPC messages and leaves
//! framing to an [`AcpTransport`]. `NdjsonTransport` serves agents spoken to over a child
//! process's stdio pipes, while an already-parsed transport (such as plugin IPC) avoids
//! re-serializing messages that were never bytes to begin with.

mod client;
mod error;
mod events;
mod frame;
mod peer;
mod pending;
mod trace;
mod transport;

#[cfg(test)]
mod tests;

pub use client::{AcpClient, PendingSessionRequest};
pub use error::AcpError;
pub use events::{AcpInboundEvent, PermissionRequest, SessionResponse};
pub use peer::AcpPeer;
pub use trace::SessionTraceRegistration;
pub use transport::{AcpMessages, AcpTransport, NdjsonTransport};
