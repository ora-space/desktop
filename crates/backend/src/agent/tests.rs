use super::AgentApi;
use crate::clock::SystemClock;
use ora_contracts::*;
use ora_db::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
use ora_logging::with_trace_logging;
use pretty_assertions::assert_eq;

/// The configurable-agent interface needs storage, not a process supervisor or plugin host.
#[test]
fn catalog_round_trip_and_public_failures_need_no_application_runtime() {
    with_trace_logging(|| {
        let temporary = tempfile::tempdir().expect("agent fixture");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(temporary.path().join("ora.sqlite3")),
                &default_migration_catalog().expect("catalog"),
            )
            .expect("database");
        let agents = AgentApi::new(pool, SystemClock);
        let created = agents
            .create(CreateAgentRequest {
                name: "Reviewer".to_string(),
                description: "Reviews changes".to_string(),
                content: Some("Review carefully.".to_string()),
            })
            .expect("create agent")
            .agent;
        assert_eq!(
            agents.list(ListAgentsRequest {}).expect("list agents"),
            ListAgentsResponse {
                agents: vec![created.clone()]
            }
        );
        assert_eq!(
            agents
                .get(GetAgentRequest {
                    agent_id: created.id.clone()
                })
                .expect("get agent"),
            GetAgentResponse {
                agent: AgentDetails {
                    id: created.id.clone(),
                    namespace: created.namespace,
                    name: created.name,
                    description: created.description,
                    content: "Review carefully.".to_string(),
                }
            }
        );
        assert_eq!(
            agents
                .delete(DeleteAgentRequest {
                    agent_id: created.id.clone()
                })
                .expect("delete agent"),
            DeleteAgentResponse {
                agent_id: created.id.clone()
            }
        );
        assert_eq!(
            agents
                .get(GetAgentRequest {
                    agent_id: created.id
                })
                .expect_err("deleted agent is unavailable")
                .public_error(),
            &PublicError::AgentNotFound(EmptyErrorParams {})
        );
    });
}
