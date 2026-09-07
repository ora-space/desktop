//! Endpoint declarations for the developerMode client namespace.

use crate::frontend::{FrontendEndpoint, FrontendResponseMode};

const NAMESPACE: &str = "developerMode";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "getDeveloperMode",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetDeveloperModeRequest",
        response_type: "DeveloperModeResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "setDeveloperMode",
        namespace: NAMESPACE,
        member_name: "set",
        request_type: "SetDeveloperModeRequest",
        response_type: "DeveloperModeResponse",
        response_mode: FrontendResponseMode::Unary,
    },
];
