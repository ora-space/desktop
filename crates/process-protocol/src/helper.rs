use serde::{Deserialize, Serialize};

/// One inspection-only helper request; launch authority cannot be represented in this protocol.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HelperRequest {
    pub version: u32,
    pub operation: HelperOperation,
}

/// Operations available before privileged workload launch has been implemented.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperOperation {
    Inspect,
}

/// The reply describes this management exchange, never a containment guarantee.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HelperResponse {
    pub version: u32,
    pub status: HelperStatus,
}

/// Explicit rejections prevent inspection success from being mistaken for launch readiness.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HelperStatus {
    Unauthorized,
    InvalidRequest,
    UnsupportedVersion,
    LaunchUnavailable,
}
