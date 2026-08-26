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
];
