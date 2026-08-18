//! Endpoint declarations for the spec generated-client namespace.

use crate::frontend::FrontendEndpoint;

const NAMESPACE: &str = "spec";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "getSpecCatalog",
        namespace: NAMESPACE,
        member_name: "catalog",
        request_type: "GetSpecCatalogRequest",
        response_type: "SpecCatalogResponse",
    },
    FrontendEndpoint {
        operation_name: "readSpec",
        namespace: NAMESPACE,
        member_name: "read",
        request_type: "ReadSpecRequest",
        response_type: "ReadSpecResponse",
    },
    FrontendEndpoint {
        operation_name: "watchSpecs",
        namespace: NAMESPACE,
        member_name: "watch",
        request_type: "WatchSpecsRequest",
        response_type: "WorkspaceFileEventBatch",
    },
];
