//! Endpoint declarations for the skill generated-client namespace.

use crate::frontend::{FrontendEndpoint, FrontendHttpMethod, NO_PATH_PARAMS, SKILL_PATH_PARAMS};
use ora_contracts::{SKILL_PATH, SKILLS_PATH};

const NAMESPACE: &str = "skill";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "createSkill",
        namespace: NAMESPACE,
        member_name: "create",
        method: FrontendHttpMethod::Post,
        path_template: SKILLS_PATH,
        request_type: "CreateSkillRequest",
        response_type: "CreateSkillResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "getSkill",
        namespace: NAMESPACE,
        member_name: "get",
        method: FrontendHttpMethod::Get,
        path_template: SKILL_PATH,
        request_type: "GetSkillRequest",
        response_type: "GetSkillResponse",
        path_params: SKILL_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "listSkills",
        namespace: NAMESPACE,
        member_name: "list",
        method: FrontendHttpMethod::Get,
        path_template: SKILLS_PATH,
        request_type: "ListSkillsRequest",
        response_type: "ListSkillsResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "updateSkill",
        namespace: NAMESPACE,
        member_name: "update",
        method: FrontendHttpMethod::Put,
        path_template: SKILL_PATH,
        request_type: "UpdateSkillRequest",
        response_type: "UpdateSkillResponse",
        path_params: SKILL_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "deleteSkill",
        namespace: NAMESPACE,
        member_name: "delete",
        method: FrontendHttpMethod::Delete,
        path_template: SKILL_PATH,
        request_type: "DeleteSkillRequest",
        response_type: "DeleteSkillResponse",
        path_params: SKILL_PATH_PARAMS,
        has_json_body: false,
    },
];
