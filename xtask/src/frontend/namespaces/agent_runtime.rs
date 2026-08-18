//! Endpoint declarations for the agentRuntime generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "agentRuntime";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[FrontendEndpoint {
    operation_name: "getAgentRuntimeStatus",
    namespace: NAMESPACE,
    member_name: "getStatus",
    request_type: "GetAgentRuntimeStatusRequest",
    response_type: "GetAgentRuntimeStatusResponse",
}];
