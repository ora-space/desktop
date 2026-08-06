//! Endpoint declarations for the skillImport generated-client namespace.

use crate::frontend::{
    FrontendEndpoint, FrontendHttpMethod, NO_PATH_PARAMS, SKILL_IMPORT_PATH_PARAMS,
};
use ora_contracts::{SKILL_IMPORT_COMMIT_PATH, SKILL_IMPORT_PATH, SKILL_IMPORTS_PATH};

const NAMESPACE: &str = "skillImport";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "prepareSkillImport",
        namespace: NAMESPACE,
        member_name: "prepare",
        method: FrontendHttpMethod::Post,
        path_template: SKILL_IMPORTS_PATH,
        request_type: "PrepareSkillImportRequest",
        response_type: "PrepareSkillImportResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "getSkillImport",
        namespace: NAMESPACE,
        member_name: "get",
        method: FrontendHttpMethod::Get,
        path_template: SKILL_IMPORT_PATH,
        request_type: "GetSkillImportSessionRequest",
        response_type: "GetSkillImportSessionResponse",
        path_params: SKILL_IMPORT_PATH_PARAMS,
        has_json_body: false,
    },
    FrontendEndpoint {
        operation_name: "commitSkillImport",
        namespace: NAMESPACE,
        member_name: "commit",
        method: FrontendHttpMethod::Post,
        path_template: SKILL_IMPORT_COMMIT_PATH,
        request_type: "CommitSkillImportRequest",
        response_type: "CommitSkillImportResponse",
        path_params: SKILL_IMPORT_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "cancelSkillImport",
        namespace: NAMESPACE,
        member_name: "cancel",
        method: FrontendHttpMethod::Delete,
        path_template: SKILL_IMPORT_PATH,
        request_type: "CancelSkillImportRequest",
        response_type: "CancelSkillImportResponse",
        path_params: SKILL_IMPORT_PATH_PARAMS,
        has_json_body: false,
    },
];
