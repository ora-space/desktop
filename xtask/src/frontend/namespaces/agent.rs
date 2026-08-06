//! Endpoint declarations for the agent generated-client namespace.

use crate::frontend::{AGENT_PATH_PARAMS, FrontendEndpoint, FrontendHttpMethod, NO_PATH_PARAMS};
use ora_contracts::{AGENT_PATH, AGENTS_PATH};

const NAMESPACE: &str = "agent";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "createAgent",
        namespace: NAMESPACE,
        member_name: "create",
        method: FrontendHttpMethod::Post,
        path_template: AGENTS_PATH,
        request_type: "CreateAgentRequest",
        response_type: "CreateAgentResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "getAgent",
        namespace: NAMESPACE,
        member_name: "get",
        method: FrontendHttpMethod::Get,
        path_template: AGENT_PATH,
        request_type: "GetAgentRequest",
        response_type: "GetAgentResponse",
        path_params: AGENT_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listAgents",
        namespace: NAMESPACE,
        member_name: "list",
        method: FrontendHttpMethod::Get,
        path_template: AGENTS_PATH,
        request_type: "ListAgentsRequest",
        response_type: "ListAgentsResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "updateAgent",
        namespace: NAMESPACE,
        member_name: "update",
        method: FrontendHttpMethod::Put,
        path_template: AGENT_PATH,
        request_type: "UpdateAgentRequest",
        response_type: "UpdateAgentResponse",
        path_params: AGENT_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "deleteAgent",
        namespace: NAMESPACE,
        member_name: "delete",
        method: FrontendHttpMethod::Delete,
        path_template: AGENT_PATH,
        request_type: "DeleteAgentRequest",
        response_type: "DeleteAgentResponse",
        path_params: AGENT_PATH_PARAMS,
        has_json_body: false,
    },
];
