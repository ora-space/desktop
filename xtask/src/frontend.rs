//! Generation-only frontend endpoint metadata used by the contract exporter.
pub(crate) const PROJECT_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "project_id",
    wire_name: "projectId",
};
pub(crate) const TASK_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "task_id",
    wire_name: "taskId",
};
pub(crate) const COMMENT_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "comment_id",
    wire_name: "commentId",
};
pub(crate) const SESSION_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "session_id",
    wire_name: "sessionId",
};
pub(crate) const SKILL_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "skill_id",
    wire_name: "skillId",
};
pub(crate) const SKILL_IMPORT_SESSION_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "session_id",
    wire_name: "sessionId",
};
pub(crate) const AGENT_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "agent_id",
    wire_name: "agentId",
};
pub(crate) const WORKFLOW_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "workflow_id",
    wire_name: "workflowId",
};
pub(crate) const WORKFLOW_VERSION_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "version",
    wire_name: "version",
};
pub(crate) const WORKFLOW_RUN_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "run_id",
    wire_name: "runId",
};
pub(crate) const WORKFLOW_SNAPSHOT_ID_PATH_PARAM: FrontendPathParam = FrontendPathParam {
    rust_field_name: "snapshot_id",
    wire_name: "snapshotId",
};
pub(crate) const WORKFLOW_RUN_PROJECT_QUERY_PARAM: FrontendQueryParam = FrontendQueryParam {
    rust_field_name: "project_id",
    wire_name: "projectId",
};
pub(crate) const WORKFLOW_RUN_WORKFLOW_QUERY_PARAM: FrontendQueryParam = FrontendQueryParam {
    rust_field_name: "workflow_id",
    wire_name: "workflowId",
};
pub(crate) const FILE_SYSTEM_DIRECTORY_PATH_QUERY_PARAM: FrontendQueryParam = FrontendQueryParam {
    rust_field_name: "path",
    wire_name: "path",
};
pub(crate) const TASK_DIFF_SCOPE_QUERY_PARAM: FrontendQueryParam = FrontendQueryParam {
    rust_field_name: "scope",
    wire_name: "scope",
};

pub(crate) const PROJECT_PATH_PARAMS: &[FrontendPathParam] = &[PROJECT_ID_PATH_PARAM];
pub(crate) const TASK_PATH_PARAMS: &[FrontendPathParam] = &[TASK_ID_PATH_PARAM];
pub(crate) const TASK_COMMENT_PATH_PARAMS: &[FrontendPathParam] =
    &[TASK_ID_PATH_PARAM, COMMENT_ID_PATH_PARAM];
pub(crate) const SESSION_PATH_PARAMS: &[FrontendPathParam] = &[SESSION_ID_PATH_PARAM];
pub(crate) const SKILL_PATH_PARAMS: &[FrontendPathParam] = &[SKILL_ID_PATH_PARAM];
pub(crate) const SKILL_IMPORT_PATH_PARAMS: &[FrontendPathParam] =
    &[SKILL_IMPORT_SESSION_ID_PATH_PARAM];
pub(crate) const AGENT_PATH_PARAMS: &[FrontendPathParam] = &[AGENT_ID_PATH_PARAM];
pub(crate) const NO_PATH_PARAMS: &[FrontendPathParam] = &[];
pub(crate) const WORKFLOW_PATH_PARAMS: &[FrontendPathParam] = &[WORKFLOW_ID_PATH_PARAM];
pub(crate) const WORKFLOW_VERSION_PATH_PARAMS: &[FrontendPathParam] =
    &[WORKFLOW_ID_PATH_PARAM, WORKFLOW_VERSION_PATH_PARAM];
pub(crate) const WORKFLOW_RUN_PATH_PARAMS: &[FrontendPathParam] = &[WORKFLOW_RUN_ID_PATH_PARAM];
pub(crate) const WORKFLOW_SNAPSHOT_PATH_PARAMS: &[FrontendPathParam] =
    &[WORKFLOW_SNAPSHOT_ID_PATH_PARAM];
pub(crate) const WORKFLOW_RUN_PROJECT_QUERY_PARAMS: &[FrontendQueryParam] =
    &[WORKFLOW_RUN_PROJECT_QUERY_PARAM];
pub(crate) const WORKFLOW_RUN_WORKFLOW_QUERY_PARAMS: &[FrontendQueryParam] =
    &[WORKFLOW_RUN_WORKFLOW_QUERY_PARAM];
pub(crate) const FILE_SYSTEM_DIRECTORY_QUERY_PARAMS: &[FrontendQueryParam] =
    &[FILE_SYSTEM_DIRECTORY_PATH_QUERY_PARAM];
pub(crate) const TASK_DIFF_QUERY_PARAMS: &[FrontendQueryParam] = &[TASK_DIFF_SCOPE_QUERY_PARAM];
pub(crate) const NO_QUERY_PARAMS: &[FrontendQueryParam] = &[];

/// Enumerates the HTTP methods supported by the generated frontend SDK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrontendHttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

/// Selects whether an endpoint returns one value or an ordered event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrontendResponseMode {
    Unary,
    Stream,
}

/// Describes one request field that the transport must interpolate into the URL path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrontendPathParam {
    pub(crate) rust_field_name: &'static str,
    pub(crate) wire_name: &'static str,
}

/// Describes one optional request field serialized into an endpoint query string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrontendQueryParam {
    pub(crate) rust_field_name: &'static str,
    pub(crate) wire_name: &'static str,
}

/// Describes one frontend-facing HTTP operation consumed by the TypeScript generator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrontendEndpoint {
    pub(crate) operation_name: &'static str,
    pub(crate) namespace: &'static str,
    pub(crate) member_name: &'static str,
    pub(crate) method: FrontendHttpMethod,
    pub(crate) path_template: &'static str,
    pub(crate) request_type: &'static str,
    pub(crate) response_type: &'static str,
    pub(crate) path_params: &'static [FrontendPathParam],
    pub(crate) has_json_body: bool,
}

impl FrontendEndpoint {
    /// Returns optional query parameters without forcing unrelated endpoints to repeat empty metadata.
    pub(crate) fn query_params(&self) -> &'static [FrontendQueryParam] {
        match self.operation_name {
            "listDirectory" => FILE_SYSTEM_DIRECTORY_QUERY_PARAMS,
            "getTaskDiff" => TASK_DIFF_QUERY_PARAMS,
            "listWorkflowRuns" => WORKFLOW_RUN_PROJECT_QUERY_PARAMS,
            "listWorkflowRunsByWorkflow" => WORKFLOW_RUN_WORKFLOW_QUERY_PARAMS,
            _ => NO_QUERY_PARAMS,
        }
    }

    /// Returns the transport mode explicitly owned by the Rust endpoint catalog.
    pub(crate) fn response_mode(&self) -> FrontendResponseMode {
        match self.operation_name {
            "loadSession" | "promptSession" | "watchWorkspace" | "watchSpecs" => {
                FrontendResponseMode::Stream
            }
            _ => FrontendResponseMode::Unary,
        }
    }
}

/// Builds the generation-only endpoint catalog from its namespace modules.
///
/// The exporter runs infrequently, so flattening the per-namespace slices here keeps each
/// namespace's route declarations local without adding a runtime catalog to ora-contracts.
pub(crate) fn frontend_endpoints() -> Vec<FrontendEndpoint> {
    namespaces::frontend_endpoints()
}

mod namespaces;

#[cfg(test)]
mod tests {
    use super::{
        FrontendEndpoint, FrontendHttpMethod, FrontendPathParam, FrontendQueryParam,
        frontend_endpoints,
    };
    use ora_contracts::TASK_PATH;
    use pretty_assertions::assert_eq;
    use std::collections::BTreeSet;

    /// Verifies directory listing paths are encoded as optional GET query parameters.
    #[test]
    fn exposes_directory_query_parameter_metadata() {
        let endpoints = frontend_endpoints();
        let list_directory = endpoints
            .iter()
            .find(|endpoint| endpoint.operation_name == "listDirectory")
            .unwrap_or_else(|| panic!("missing listDirectory endpoint"));

        assert_eq!(
            list_directory.query_params(),
            &[FrontendQueryParam {
                rust_field_name: "path",
                wire_name: "path",
            }]
        );
    }

    /// Verifies update operations describe the path/body split needed by the generated client.
    #[test]
    fn preserves_path_params_for_update_routes() {
        let update_task = frontend_endpoints()
            .iter()
            .find(|endpoint| endpoint.operation_name == "updateTask")
            .copied()
            .unwrap_or_else(|| panic!("missing updateTask endpoint"));

        assert_eq!(
            update_task,
            FrontendEndpoint {
                operation_name: "updateTask",
                namespace: "task",
                member_name: "update",
                method: FrontendHttpMethod::Put,
                path_template: TASK_PATH,
                request_type: "UpdateTaskRequest",
                response_type: "UpdateTaskResponse",
                path_params: &[FrontendPathParam {
                    rust_field_name: "task_id",
                    wire_name: "taskId",
                }],
                has_json_body: true,
            }
        );
    }

    /// Verifies every namespace member is unique so no operation is shadowed on the generated client.
    #[test]
    fn exports_unique_namespace_members() {
        let mut seen_members = BTreeSet::new();

        for endpoint in frontend_endpoints() {
            assert_eq!(
                seen_members.insert((endpoint.namespace, endpoint.member_name)),
                true,
                "duplicate client member {}.{}",
                endpoint.namespace,
                endpoint.member_name
            );
        }
    }

    /// Verifies the exported endpoint manifest omits backend-owned worktree operations.
    #[test]
    fn omits_worktree_endpoints_from_frontend_manifest() {
        assert_eq!(
            frontend_endpoints()
                .iter()
                .all(|endpoint| !endpoint.operation_name.contains("Worktree")),
            true
        );
    }

    /// Verifies catalogs publish separate collection and identifier resource routes.
    #[test]
    fn exports_skill_and_agent_crud_endpoints() {
        assert!(
            frontend_endpoints()
                .iter()
                .any(|endpoint| endpoint.operation_name == "updateSkill"
                    && endpoint.path_template == "/api/skills/{skillId}")
        );
        assert!(
            frontend_endpoints()
                .iter()
                .any(|endpoint| endpoint.operation_name == "updateAgent"
                    && endpoint.path_template == "/api/agents/{agentId}")
        );
    }
}
