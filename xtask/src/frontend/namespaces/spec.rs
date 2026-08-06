//! Endpoint declarations for the spec generated-client namespace.

use crate::frontend::{FrontendEndpoint, FrontendHttpMethod, NO_PATH_PARAMS, PROJECT_PATH_PARAMS};
use ora_contracts::{
    PROJECT_SPEC_SOURCES_PATH, SPEC_CATALOG_PATH, SPEC_READ_PATH, SPEC_RESOLVE_SOURCE_PATH,
    SPEC_WATCH_PATH,
};

const NAMESPACE: &str = "spec";

pub(super) const ENDPOINTS: &[FrontendEndpoint] = &[
    FrontendEndpoint {
        operation_name: "getSpecCatalog",
        namespace: NAMESPACE,
        member_name: "catalog",
        method: FrontendHttpMethod::Post,
        path_template: SPEC_CATALOG_PATH,
        request_type: "GetSpecCatalogRequest",
        response_type: "SpecCatalogResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "readSpec",
        namespace: NAMESPACE,
        member_name: "read",
        method: FrontendHttpMethod::Post,
        path_template: SPEC_READ_PATH,
        request_type: "ReadSpecRequest",
        response_type: "ReadSpecResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "resolveSpecSource",
        namespace: NAMESPACE,
        member_name: "resolveSource",
        method: FrontendHttpMethod::Post,
        path_template: SPEC_RESOLVE_SOURCE_PATH,
        request_type: "ResolveSpecSourceRequest",
        response_type: "ResolveSpecSourceResponse",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "updateProjectSpecSources",
        namespace: NAMESPACE,
        member_name: "updateProjectSources",
        method: FrontendHttpMethod::Put,
        path_template: PROJECT_SPEC_SOURCES_PATH,
        request_type: "UpdateProjectSpecSourcesRequest",
        response_type: "UpdateProjectSpecSourcesResponse",
        path_params: PROJECT_PATH_PARAMS,
        has_json_body: true,
    },
    FrontendEndpoint {
        operation_name: "watchSpecs",
        namespace: NAMESPACE,
        member_name: "watch",
        method: FrontendHttpMethod::Post,
        path_template: SPEC_WATCH_PATH,
        request_type: "WatchSpecsRequest",
        response_type: "WorkspaceFileEventBatch",
        path_params: NO_PATH_PARAMS,
        has_json_body: true,
    },
];
