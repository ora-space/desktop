//! Endpoint declarations for the gitIdentity generated-client namespace.

use crate::frontend::{FrontendEndpoint, FrontendHttpMethod, NO_PATH_PARAMS};
use ora_contracts::GIT_IDENTITY_PATH;

const NAMESPACE: &str = "gitIdentity";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[FrontendEndpoint {
    operation_name: "getGitIdentity",
    namespace: NAMESPACE,
    member_name: "get",
    method: FrontendHttpMethod::Get,
    path_template: GIT_IDENTITY_PATH,
    request_type: "GetGitIdentityRequest",
    response_type: "GitIdentityResponse",
    path_params: NO_PATH_PARAMS,
    has_json_body: false,
}];
