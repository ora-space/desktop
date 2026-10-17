//! Endpoint declarations for generic Effect target status and recovery control.

use crate::frontend::{FrontendEndpoint, FrontendResponseMode};

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "getEffectTargetStatus",
        namespace: "effect",
        member_name: "getTargetStatus",
        request_type: "GetEffectTargetStatusRequest",
        response_type: "GetEffectTargetStatusResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "recoverEffectTarget",
        namespace: "effect",
        member_name: "recoverTarget",
        request_type: "RecoverEffectTargetRequest",
        response_type: "RecoverEffectTargetResponse",
        response_mode: FrontendResponseMode::Unary,
    },
];
