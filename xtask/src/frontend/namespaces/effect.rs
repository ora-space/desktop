//! Endpoint declarations for the effect client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "effect";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[FrontendEndpoint {
    operation_name: "getMcpApplicationState",
    namespace: NAMESPACE,
    member_name: "getMcpApplicationState",
    request_type: "GetMcpApplicationStateRequest",
    response_type: "GetMcpApplicationStateResponse",
}];
