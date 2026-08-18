//! Endpoint declarations for the skillImport generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "skillImport";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "prepareSkillImport",
        namespace: NAMESPACE,
        member_name: "prepare",
        request_type: "PrepareSkillImportRequest",
        response_type: "PrepareSkillImportResponse",
    },
    FrontendEndpoint {
        operation_name: "getSkillImport",
        namespace: NAMESPACE,
        member_name: "get",
        request_type: "GetSkillImportSessionRequest",
        response_type: "GetSkillImportSessionResponse",
    },
    FrontendEndpoint {
        operation_name: "commitSkillImport",
        namespace: NAMESPACE,
        member_name: "commit",
        request_type: "CommitSkillImportRequest",
        response_type: "CommitSkillImportResponse",
    },
    FrontendEndpoint {
        operation_name: "cancelSkillImport",
        namespace: NAMESPACE,
        member_name: "cancel",
        request_type: "CancelSkillImportRequest",
        response_type: "CancelSkillImportResponse",
    },
];
