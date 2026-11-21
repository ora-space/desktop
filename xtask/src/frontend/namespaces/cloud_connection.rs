//! Endpoint declarations for the Desktop Cloud connection namespace.

use crate::frontend::{FrontendEndpoint, FrontendResponseMode};

const NAMESPACE: &str = "cloudConnection";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "getCloudConnection",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetCloudConnectionRequest",
        response_type: "CloudConnection",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "setCloudServer",
        namespace: NAMESPACE,
        member_name: "setServer",
        request_type: "SetCloudServerRequest",
        response_type: "CloudConnection",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "startCloudLogin",
        namespace: NAMESPACE,
        member_name: "startLogin",
        request_type: "StartCloudLoginRequest",
        response_type: "CloudConnection",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "cancelCloudLogin",
        namespace: NAMESPACE,
        member_name: "cancelLogin",
        request_type: "CancelCloudLoginRequest",
        response_type: "CloudConnection",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "logoutCloud",
        namespace: NAMESPACE,
        member_name: "logout",
        request_type: "LogoutCloudRequest",
        response_type: "LogoutCloudResponse",
        response_mode: FrontendResponseMode::Unary,
    },
];
