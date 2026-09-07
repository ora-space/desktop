use super::Effects;
use ora_contracts::{
    EmptyErrorParams, GetEffectTargetStatusRequest, GetEffectTargetStatusResponse, PublicError,
};
use ora_db::{DatabaseBootstrapper, DatabaseLocation, default_migration_catalog};
use pretty_assertions::assert_eq;

/// Missing targets are an empty status, while actual SQLite read failures remain public errors;
/// neither path needs a plugin host, worker, or supervisor to exercise the consumer interface.
#[test]
fn missing_targets_and_storage_failures_stay_distinct_without_runtime_composition() {
    ora_logging::with_trace_logging(|| {
        let temporary = tempfile::tempdir().expect("Effect status fixture");
        let path = temporary.path().join("ora.sqlite3");
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(&path),
                &default_migration_catalog().expect("catalog"),
            )
            .expect("database");
        let effects = Effects::new(pool);
        let requests = [
            GetEffectTargetStatusRequest::Target {
                target_id: "missing-target".to_string(),
            },
            GetEffectTargetStatusRequest::WorkspaceAgent {
                workspace_id: "missing-workspace".to_string(),
                agent_plugin_id: "official/ora-space.fixture".to_string(),
            },
        ];
        for request in &requests {
            assert_eq!(
                effects
                    .target_status(request.clone())
                    .expect("missing status"),
                GetEffectTargetStatusResponse { status: None }
            );
        }
        rusqlite::Connection::open(&path)
            .expect("fault injection connection")
            .execute_batch("DROP TABLE effect_target_status;")
            .expect("inject storage read failure");
        let error = effects
            .target_status(requests[0].clone())
            .expect_err("missing table is not a missing target");
        assert_eq!(
            error.public_error(),
            &PublicError::InternalError(EmptyErrorParams {})
        );
        assert!(std::error::Error::source(&error).is_some());
    });
}
