//! Desktop-owned native route shared by the Cloud connection emitter and contract exporter.

/// Carries the whole `CloudConnection` after every change, the same value the unary get returns.
pub const CHANGED_EVENT: &str = "cloud-connection-changed";
