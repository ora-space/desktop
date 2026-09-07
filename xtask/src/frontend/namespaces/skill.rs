//! Endpoint declarations for the skill generated-client namespace.

use crate::frontend::{FrontendEndpoint, FrontendResponseMode};

const NAMESPACE: &str = "skill";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "createSkill",
        namespace: NAMESPACE,
        member_name: "create",
        request_type: "CreateSkillRequest",
        response_type: "CreateSkillResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "getSkill",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetSkillRequest",
        response_type: "GetSkillResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "listSkills",
        namespace: NAMESPACE,
        member_name: "list",
        request_type: "ListSkillsRequest",
        response_type: "ListSkillsResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "updateSkill",
        namespace: NAMESPACE,
        member_name: "update",
        request_type: "UpdateSkillRequest",
        response_type: "UpdateSkillResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "deleteSkill",
        namespace: NAMESPACE,
        member_name: "delete",
        request_type: "DeleteSkillRequest",
        response_type: "DeleteSkillResponse",
        response_mode: FrontendResponseMode::Unary,
    },
];
