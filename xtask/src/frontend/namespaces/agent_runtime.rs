//! Endpoint declarations for the agentRuntime generated-client namespace.

use crate::frontend::{FrontendEndpoint, FrontendHttpMethod, NO_PATH_PARAMS};
use ora_contracts::AGENT_RUNTIME_STATUS_PATH;

const NAMESPACE: &str = "agentRuntime";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[FrontendEndpoint {
    operation_name: "getAgentRuntimeStatus",
    namespace: NAMESPACE,
    member_name: "getStatus",
    method: FrontendHttpMethod::Get,
    path_template: AGENT_RUNTIME_STATUS_PATH,
    request_type: "GetAgentRuntimeStatusRequest",
    response_type: "GetAgentRuntimeStatusResponse",
    path_params: NO_PATH_PARAMS,
    has_json_body: false,
}];
