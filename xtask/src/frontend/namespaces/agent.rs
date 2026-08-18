//! Endpoint declarations for the agent generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "agent";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "createAgent",
        namespace: NAMESPACE,
        member_name: "create",
        request_type: "CreateAgentRequest",
        response_type: "CreateAgentResponse",
    },
    FrontendEndpoint {
        operation_name: "getAgent",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetAgentRequest",
        response_type: "GetAgentResponse",
    },
    FrontendEndpoint {
        operation_name: "listAgents",
        namespace: NAMESPACE,
        member_name: "list",
        request_type: "ListAgentsRequest",
        response_type: "ListAgentsResponse",
    },
    FrontendEndpoint {
        operation_name: "updateAgent",
        namespace: NAMESPACE,
        member_name: "update",
        request_type: "UpdateAgentRequest",
        response_type: "UpdateAgentResponse",
    },
    FrontendEndpoint {
        operation_name: "deleteAgent",
        namespace: NAMESPACE,
        member_name: "delete",
        request_type: "DeleteAgentRequest",
        response_type: "DeleteAgentResponse",
    },
];
