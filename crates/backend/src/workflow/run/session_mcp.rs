//! Restores node-local MCP intent from the existing durable workflow/session relationship.

use crate::BackendError;
use crate::session_setup::{SessionMcpSelection, SessionMcpSelectionSource};
use ora_application::{WorkflowGraph, WorkflowRunEngineRepository};
use ora_db::{RepositoryPool, SqliteWorkflowRunEngineRepository};
use ora_domain::SessionId;

/// Reads node-local intent without introducing workflow dependencies into the generic runtime.
pub(crate) struct WorkflowSessionMcpSelectionSource {
    repository: SqliteWorkflowRunEngineRepository,
}

impl WorkflowSessionMcpSelectionSource {
    /// Captures the durable repository used when a session actor is recreated or rebound.
    pub(crate) fn new(pool: RepositoryPool) -> Self {
        Self {
            repository: SqliteWorkflowRunEngineRepository::new(pool),
        }
    }
}

impl SessionMcpSelectionSource for WorkflowSessionMcpSelectionSource {
    /// Uses the frozen run graph, so editing a draft never changes an existing conversation.
    fn selection_for(&self, session_id: &SessionId) -> Result<SessionMcpSelection, BackendError> {
        let repository = &self.repository;
        let node_run = repository
            .find_node_run_by_session_id(session_id)
            .map_err(|source| {
                BackendError::internal("failed to resolve workflow session ownership", source)
            })?;
        let Some(node_run) = node_run else {
            return Ok(SessionMcpSelection::Automatic);
        };
        let context = repository
            .find_execution_context(&node_run.run_id)
            .map_err(|source| {
                BackendError::internal("failed to load frozen workflow context", source)
            })?
            .ok_or_else(|| {
                BackendError::internal(
                    "workflow session has no execution context",
                    std::io::Error::other("missing workflow context"),
                )
            })?;
        let graph = WorkflowGraph::parse(&context.graph_json)
            .map_err(|source| BackendError::internal("invalid frozen workflow graph", source))?;
        let config = graph
            .node(&node_run.node_id)
            .and_then(|node| node.agent_config.as_ref())
            .ok_or_else(|| {
                BackendError::internal(
                    "workflow session has no agent configuration",
                    std::io::Error::other("missing workflow node"),
                )
            })?;
        let ids = config
            .mcps
            .iter()
            .filter(|mcp| mcp.enabled)
            .map(|mcp| mcp.mcp_id.clone())
            .collect();
        Ok(SessionMcpSelection::Explicit(ids))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::run::test_fixture::{
        AGENT_GRAPH, bind_and_park, bootstrap, run_test, started_run,
    };
    use ora_application::WorkflowRepository;
    use ora_db::SqliteWorkflowRepository;
    use ora_domain::WorkflowId;
    use pretty_assertions::assert_eq;

    /// Recreating runtime state uses the bound published snapshot, never the mutable draft.
    #[test]
    fn restores_frozen_node_selection_and_keeps_ordinary_sessions_automatic() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let graph = AGENT_GRAPH.replace("\"prompt\":\"do\"", "\"mcps\":[{\"mcpId\":\"official/first\",\"enabled\":true},{\"mcpId\":\"official/second\",\"enabled\":false}],\"prompt\":\"do\"");
            let (_, nodes) = started_run(&temp, &pool, &graph);
            let agent = nodes.iter().find(|node| node.node_id == "agent").unwrap();
            let (session_id, _) = bind_and_park(&pool, agent);
            let source = WorkflowSessionMcpSelectionSource::new(pool.clone());
            SqliteWorkflowRepository::new(pool)
                .update_draft(&WorkflowId::new("workflow-1"), AGENT_GRAPH.to_string(), 60)
                .unwrap();
            assert_eq!(
                source.selection_for(&session_id).unwrap(),
                SessionMcpSelection::Explicit(["official/first".to_string()].into_iter().collect())
            );
            assert_eq!(
                source.selection_for(&SessionId::new("ordinary")).unwrap(),
                SessionMcpSelection::Automatic
            );
        });
    }

    /// Disabled legacy or missing IDs remain persisted but cannot widen the restored selection.
    #[test]
    fn all_disabled_bindings_restore_an_explicit_empty_selection() {
        run_test(async {
            let (temp, pool) = bootstrap();
            let graph = AGENT_GRAPH.replace(
                "\"prompt\":\"do\"",
                "\"mcps\":[{\"mcpId\":\"old-example\",\"enabled\":false}],\"prompt\":\"do\"",
            );
            let (_, nodes) = started_run(&temp, &pool, &graph);
            let agent = nodes.iter().find(|node| node.node_id == "agent").unwrap();
            let (session_id, _) = bind_and_park(&pool, agent);
            assert_eq!(
                WorkflowSessionMcpSelectionSource::new(pool)
                    .selection_for(&session_id)
                    .unwrap(),
                SessionMcpSelection::Explicit(Default::default())
            );
        });
    }
}
