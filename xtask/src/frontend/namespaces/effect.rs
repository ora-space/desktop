//! Endpoint declarations for generic Effect target status queries.

use crate::frontend::FrontendEndpoint;

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[FrontendEndpoint {
    operation_name: "getEffectTargetStatus",
    namespace: "effect",
    member_name: "getTargetStatus",
    request_type: "GetEffectTargetStatusRequest",
    response_type: "GetEffectTargetStatusResponse",
}];
