//! Endpoint declarations for the projectWorkContext generated-client namespace.

use crate::frontend::{FrontendEndpoint, FrontendHttpMethod, NO_PATH_PARAMS};
use ora_contracts::{PROJECT_WORK_CONTEXT_OPEN_PATH, PROJECT_WORK_CONTEXT_RENEW_PATH};

const NAMESPACE: &str = "projectWorkContext";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "openProjectWorkContext",
        namespace: NAMESPACE,
        member_name: "open",
        method: FrontendHttpMethod::Post,
        path_template: PROJECT_WORK_CONTEXT_OPEN_PATH,
        request_type: "OpenProjectWorkContextRequest",
        response_type: "OpenProjectWorkContextResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "renewProjectWorkContext",
        namespace: NAMESPACE,
        member_name: "renew",
        method: FrontendHttpMethod::Post,
        path_template: PROJECT_WORK_CONTEXT_RENEW_PATH,
        request_type: "RenewProjectWorkContextRequest",
        response_type: "RenewProjectWorkContextResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
];
