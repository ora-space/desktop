use std::io::{self, Cursor};
use std::path::PathBuf;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::ScopeCreationIntent;

pub const GUARDIAN_WIRE_VERSION: u16 = 3;
pub const GUARDIAN_MAX_FRAME: usize = 16_384;

/// Internal recovery material; it is not Controller authorization or a Run launch capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardianAccess {
    pub scope_dir: PathBuf,
    pub intent: ScopeCreationIntent,
}

/// Versioned material delivered only over the dedicated inherited bootstrap channel.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardianBootstrap {
    pub version: u16,
    pub access: GuardianAccess,
}

/// The channel must match the actual socket, even for this read-only bootstrap probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardianChannel {
    Control,
    Events,
    Io,
}

impl GuardianChannel {
    /// Keeps the three final endpoint names in the protocol owner.
    pub fn socket_name(self) -> &'static str {
        match self {
            Self::Control => "control.sock",
            Self::Events => "events.sock",
            Self::Io => "io.sock",
        }
    }
}

/// Readiness is a read-only probe, separate from management and Run messages.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardianReadyRequest {
    pub version: u16,
    pub intent: ScopeCreationIntent,
    pub channel: GuardianChannel,
    pub session: [u8; 16],
}

/// A live initialized original guardian, not evidence of a Run, control takeover or cleanup.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardianReady {
    pub version: u16,
    pub intent: ScopeCreationIntent,
    pub channel: GuardianChannel,
    pub session: [u8; 16],
}

/// Encodes one bounded length-prefixed MessagePack map; unknown fields are rejected on decode.
pub fn encode_guardian_frame<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    let payload = rmp_serde::to_vec_named(value)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "guardian encoding failed"))?;
    if payload.len() > GUARDIAN_MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "guardian frame exceeds limit",
        ));
    }
    let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
    frame.extend(payload);
    Ok(frame)
}

/// Requires a complete single payload and rejects trailing objects rather than ignoring them.
pub fn decode_guardian_payload<T: DeserializeOwned>(bytes: &[u8]) -> io::Result<T> {
    if bytes.len() > GUARDIAN_MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "guardian frame exceeds limit",
        ));
    }
    let mut cursor = Cursor::new(bytes);
    let mut decoder = rmp_serde::Deserializer::new(&mut cursor);
    decoder.set_max_depth(/*depth*/ 16);
    let value = T::deserialize(&mut decoder)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid guardian message"))?;
    if cursor.position() != bytes.len() as u64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "trailing guardian data",
        ));
    }
    Ok(value)
}
