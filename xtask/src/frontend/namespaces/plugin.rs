//! Endpoint declarations for the plugin generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "plugin";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[FrontendEndpoint {
    operation_name: "listInstalledPlugins",
    namespace: NAMESPACE,
    member_name: "listInstalled",
    request_type: "ListInstalledPluginsRequest",
    response_type: "ListInstalledPluginsResponse",
}];
