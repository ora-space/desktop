//! Generation-only catalog of Desktop contract operations.

/// Selects whether an operation returns one value or an ordered event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FrontendResponseMode {
    Unary,
    Stream,
}

/// Describes one operation exposed by the generated Desktop client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FrontendEndpoint {
    pub(crate) operation_name: &'static str,
    pub(crate) namespace: &'static str,
    pub(crate) member_name: &'static str,
    pub(crate) request_type: &'static str,
    pub(crate) response_type: &'static str,
}

impl FrontendEndpoint {
    /// Returns the stream mode owned by the operation catalog.
    pub(crate) fn response_mode(&self) -> FrontendResponseMode {
        match self.operation_name {
            "loadSession" | "promptSession" | "watchWorkspace" | "watchProject"
            | "watchAppEvents" => FrontendResponseMode::Stream,
            _ => FrontendResponseMode::Unary,
        }
    }
}

/// Builds the generation-only operation catalog from its namespace modules.
pub(crate) fn frontend_endpoints() -> Vec<FrontendEndpoint> {
    namespaces::frontend_endpoints()
}

mod namespaces;

#[cfg(test)]
mod tests {
    use super::{FrontendEndpoint, FrontendResponseMode, frontend_endpoints};
    use pretty_assertions::assert_eq;
    use std::collections::BTreeSet;

    /// Verifies an endpoint retains only the metadata needed by Desktop IPC.
    #[test]
    fn preserves_desktop_operation_metadata() {
        let update_task = frontend_endpoints()
            .into_iter()
            .find(|endpoint| endpoint.operation_name == "updateTask")
            .unwrap_or_else(|| panic!("missing updateTask endpoint"));

        assert_eq!(
            update_task,
            FrontendEndpoint {
                operation_name: "updateTask",
                namespace: "task",
                member_name: "update",
                request_type: "UpdateTaskRequest",
                response_type: "UpdateTaskResponse",
            }
        );
    }

    /// Verifies the response mode remains explicit for streaming operations.
    #[test]
    fn identifies_stream_operations() {
        assert_eq!(
            frontend_endpoints()
                .into_iter()
                .find(|endpoint| endpoint.operation_name == "watchAppEvents")
                .map(|endpoint| endpoint.response_mode()),
            Some(FrontendResponseMode::Stream)
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

    /// Verifies backend-owned worktree operations are not exposed by the catalog.
    #[test]
    fn omits_worktree_endpoints_from_frontend_manifest() {
        assert_eq!(
            frontend_endpoints()
                .iter()
                .all(|endpoint| !endpoint.operation_name.contains("Worktree")),
            true
        );
    }

    /// Verifies the catalog contains the expected CRUD operation names.
    #[test]
    fn exports_skill_and_agent_crud_endpoints() {
        let operations = frontend_endpoints()
            .into_iter()
            .map(|endpoint| endpoint.operation_name)
            .collect::<BTreeSet<_>>();

        assert!(operations.contains("updateSkill"));
        assert!(operations.contains("updateAgent"));
    }

    /// Verifies runtime log-level reads and writes use the generated Desktop namespace.
    #[test]
    fn exports_runtime_log_level_endpoints() {
        let endpoints = frontend_endpoints();
        let runtime_endpoints = endpoints
            .iter()
            .filter(|endpoint| endpoint.namespace == "runtimeLogLevel")
            .copied()
            .collect::<Vec<_>>();

        assert_eq!(
            runtime_endpoints,
            vec![
                FrontendEndpoint {
                    operation_name: "getRuntimeLogLevel",
                    namespace: "runtimeLogLevel",
                    member_name: "get",
                    request_type: "GetRuntimeLogLevelRequest",
                    response_type: "RuntimeLogLevelStateResponse",
                },
                FrontendEndpoint {
                    operation_name: "setRuntimeLogLevel",
                    namespace: "runtimeLogLevel",
                    member_name: "set",
                    request_type: "SetRuntimeLogLevelRequest",
                    response_type: "RuntimeLogLevelStateResponse",
                },
            ]
        );
    }

    /// Verifies developer-mode reads and writes use the generated Desktop namespace.
    #[test]
    fn exports_developer_mode_endpoints() {
        let endpoints = frontend_endpoints();
        let developer_mode_endpoints = endpoints
            .iter()
            .filter(|endpoint| endpoint.namespace == "developerMode")
            .copied()
            .collect::<Vec<_>>();

        assert_eq!(
            developer_mode_endpoints,
            vec![
                FrontendEndpoint {
                    operation_name: "getDeveloperMode",
                    namespace: "developerMode",
                    member_name: "get",
                    request_type: "GetDeveloperModeRequest",
                    response_type: "DeveloperModeResponse",
                },
                FrontendEndpoint {
                    operation_name: "setDeveloperMode",
                    namespace: "developerMode",
                    member_name: "set",
                    request_type: "SetDeveloperModeRequest",
                    response_type: "DeveloperModeResponse",
                },
            ]
        );
    }
}
