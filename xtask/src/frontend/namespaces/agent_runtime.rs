//! Endpoint declarations for the agentRuntime generated-client namespace.

use crate::frontend::{FrontendEndpoint, FrontendResponseMode};

const NAMESPACE: &str = "agentRuntime";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "getAgentRuntimeStatus",
        namespace: NAMESPACE,
        member_name: "getStatus",
        request_type: "GetAgentRuntimeStatusRequest",
        response_type: "GetAgentRuntimeStatusResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "listAgentModels",
        namespace: NAMESPACE,
        member_name: "listModels",
        request_type: "ListAgentModelsRequest",
        response_type: "ListAgentModelsResponse",
        response_mode: FrontendResponseMode::Unary,
    },
];
