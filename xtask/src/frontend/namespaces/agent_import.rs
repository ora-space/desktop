//! Endpoint declarations for the agentImport generated-client namespace.

use crate::frontend::{FrontendEndpoint, FrontendHttpMethod, NO_PATH_PARAMS};
use ora_contracts::{AGENT_IMPORT_COMMIT_PATH, AGENT_IMPORT_PREPARE_PATH};

const NAMESPACE: &str = "agentImport";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "prepareAgentImport",
        namespace: NAMESPACE,
        member_name: "prepare",
        method: FrontendHttpMethod::Post,
        path_template: AGENT_IMPORT_PREPARE_PATH,
        request_type: "PrepareAgentImportRequest",
        response_type: "PrepareAgentImportResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "commitAgentImport",
        namespace: NAMESPACE,
        member_name: "commit",
        method: FrontendHttpMethod::Post,
        path_template: AGENT_IMPORT_COMMIT_PATH,
        request_type: "CommitAgentImportRequest",
        response_type: "CommitAgentImportResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
];
