//! Endpoint declarations for the agentImport generated-client namespace.

use crate::frontend::{FrontendEndpoint, FrontendResponseMode};

const NAMESPACE: &str = "agentImport";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "prepareAgentImport",
        namespace: NAMESPACE,
        member_name: "prepare",
        request_type: "PrepareAgentImportRequest",
        response_type: "PrepareAgentImportResponse",
        response_mode: FrontendResponseMode::Unary,
    },
    FrontendEndpoint {
        operation_name: "commitAgentImport",
        namespace: NAMESPACE,
        member_name: "commit",
        request_type: "CommitAgentImportRequest",
        response_type: "CommitAgentImportResponse",
        response_mode: FrontendResponseMode::Unary,
    },
];
