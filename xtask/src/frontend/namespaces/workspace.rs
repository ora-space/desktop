//! Endpoint declarations for the workspace generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "workspace";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[FrontendEndpoint {
    operation_name: "listWorkspaces",
    namespace: NAMESPACE,
    member_name: "list",
    request_type: "ListWorkspacesRequest",
    response_type: "ListWorkspacesResponse",
}];
