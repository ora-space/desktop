//! Endpoint declarations for the gitIdentity generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "gitIdentity";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[FrontendEndpoint {
    operation_name: "getGitIdentity",
    namespace: NAMESPACE,
    member_name: "get",
    request_type: "GetGitIdentityRequest",
    response_type: "GitIdentityResponse",
}];
