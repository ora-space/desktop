//! Endpoint declarations for the network proxy settings namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "proxy";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "getProxySettings",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetProxySettingsRequest",
        response_type: "GetProxySettingsResponse",
    },
    FrontendEndpoint {
        operation_name: "setProxySettings",
        namespace: NAMESPACE,
        member_name: "set",
        request_type: "SetProxySettingsRequest",
        response_type: "SetProxySettingsResponse",
    },
    FrontendEndpoint {
        operation_name: "clearProxySettings",
        namespace: NAMESPACE,
        member_name: "clear",
        request_type: "ClearProxySettingsRequest",
        response_type: "ClearProxySettingsResponse",
    },
    FrontendEndpoint {
        operation_name: "checkProxySettings",
        namespace: NAMESPACE,
        member_name: "check",
        request_type: "CheckProxySettingsRequest",
        response_type: "CheckProxySettingsResponse",
    },
];
