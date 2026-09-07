//! Endpoint declarations for the process-wide runtimeLogLevel client namespace.

use crate::frontend::{FrontendEndpoint, FrontendResponseMode};

const NAMESPACE: &str = "runtimeLogLevel";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "getRuntimeLogLevel",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetRuntimeLogLevelRequest",
        response_type: "RuntimeLogLevelStateResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "setRuntimeLogLevel",
        namespace: NAMESPACE,
        member_name: "set",
        request_type: "SetRuntimeLogLevelRequest",
        response_type: "RuntimeLogLevelStateResponse",
        response_mode: FrontendResponseMode::Unary,
    },
];
