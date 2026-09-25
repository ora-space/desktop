use crate::{CloneIntake, CloneOperation, ControllerHandle, Error};
use axum::{
    Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    http::StatusCode,
    routing::get,
};
use ora_contracts::controller_api::*;
use ora_node_protocol::*;

struct App<S: CloneIntake> {
    controller: ControllerHandle<S>,
    node: NodeId,
}

impl<S: CloneIntake> Clone for App<S> {
    fn clone(&self) -> Self {
        Self {
            controller: self.controller.clone(),
            node: self.node.clone(),
        }
    }
}
type Failure = (StatusCode, Json<MiniError>);

/// Composes only the transitional clone surface; no Desktop bindings or Node wire messages leak through HTTP.
pub(super) fn router<S: CloneIntake>(controller: ControllerHandle<S>, node: NodeId) -> Router {
    Router::new()
        .route("/api/clones", get(list::<S>).post(submit::<S>))
        .route("/api/clones/{execution}", get(detail::<S>))
        .with_state(App { controller, node })
}

/// Maps adapter errors without turning a failed HTTP call into a failed clone result.
fn failure(error: Error) -> Failure {
    let (status, code) = match error {
        Error::Conflict | Error::AlreadyRunning => (StatusCode::CONFLICT, MiniErrorCode::Conflict),
        Error::Validation(_) => (StatusCode::BAD_REQUEST, MiniErrorCode::InvalidInput),
        Error::Io(_)
        | Error::Sql(_)
        | Error::Encoding(_)
        | Error::InvalidStorage
        | Error::Injected
        | Error::Configuration(_)
        | Error::Unavailable(_)
        | Error::Unknown(_)
        | Error::StaleEligibility => (StatusCode::SERVICE_UNAVAILABLE, MiniErrorCode::Unavailable),
    };
    (status, Json(MiniError { code }))
}

/// Returns acceptance only after Controller commits the original request identity and full intent.
async fn submit<S: CloneIntake>(
    State(app): State<App<S>>,
    input: Result<Json<MiniCloneRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<MiniCloneAccepted>), Failure> {
    let Json(input) = input.map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(MiniError {
                code: MiniErrorCode::InvalidInput,
            }),
        )
    })?;
    let repository = CloneRepositoryUrl::parse(&input.repository).map_err(|_| {
        (
            StatusCode::BAD_REQUEST,
            Json(MiniError {
                code: MiniErrorCode::InvalidInput,
            }),
        )
    })?;
    let command = app
        .controller
        .accept_clone(
            RequestId::new(input.request_id.clone()),
            CloneExecutionSpec {
                node_id: app.node,
                repository,
                branch: BranchName::new(input.branch),
            },
        )
        .await
        .map_err(failure)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(MiniCloneAccepted {
            request_id: input.request_id,
            operation_id: command.operation_id.as_str().into(),
            execution_id: command.execution_id.as_str().into(),
        }),
    ))
}

/// Lists durable intent regardless of current Node connectivity.
async fn list<S: CloneIntake>(
    State(app): State<App<S>>,
) -> Result<Json<Vec<MiniCloneOperation>>, Failure> {
    Ok(Json(
        app.controller
            .operations()
            .await
            .map_err(failure)?
            .into_iter()
            .map(present)
            .collect(),
    ))
}

/// An absent execution is not the same as a pending result.
async fn detail<S: CloneIntake>(
    State(app): State<App<S>>,
    Path(execution): Path<String>,
) -> Result<Json<MiniCloneOperation>, Failure> {
    app.controller
        .operation(ExecutionId::new(execution))
        .await
        .map_err(failure)?
        .map(present)
        .map(Json)
        .ok_or((
            StatusCode::NOT_FOUND,
            Json(MiniError {
                code: MiniErrorCode::NotFound,
            }),
        ))
}

/// Projects immutable Controller facts; pending deliberately makes no live-progress claim.
fn present(operation: CloneOperation) -> MiniCloneOperation {
    let state = match operation.result {
        None => MiniCloneState::Pending,
        Some(CloneExecutionResult::CloneReady(result)) => MiniCloneState::Succeeded {
            path: result.path.as_str().into(),
            commit: result.commit.as_str().into(),
        },
        Some(CloneExecutionResult::CloneFailed(result)) => MiniCloneState::Failed {
            reason: match result.failure {
                CloneFailureCode::SourceUnavailable => MiniCloneFailure::SourceUnavailable,
                CloneFailureCode::BranchNotFound => MiniCloneFailure::BranchNotFound,
                CloneFailureCode::DestinationConflict => MiniCloneFailure::DestinationConflict,
                CloneFailureCode::OperationFailed => MiniCloneFailure::OperationFailed,
                CloneFailureCode::Interrupted => MiniCloneFailure::Interrupted,
            },
            retained_path: match result.residual {
                CloneResidual::NoDirectory {} => None,
                CloneResidual::Retained { path, .. } => Some(path.as_str().into()),
            },
        },
    };
    let command = operation.command;
    MiniCloneOperation {
        operation_id: command.operation_id.as_str().into(),
        execution_id: command.execution_id.as_str().into(),
        node_id: command.payload.spec.node_id.as_str().into(),
        repository: command.payload.spec.repository.as_str().into(),
        branch: command.payload.spec.branch.as_str().into(),
        state,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// An interrupted attempt keeps its own browser reason and retained path instead of reading
    /// as a Git failure, so the caller knows a plain retry is appropriate.
    #[test]
    fn interrupted_failure_presents_its_own_reason() {
        let spec = CloneExecutionSpec {
            node_id: NodeId::new("node"),
            repository: CloneRepositoryUrl::parse("https://example.test/repo.git").unwrap(),
            branch: BranchName::new("main"),
        };
        let operation = CloneOperation {
            command: CloneRepositoryMessage {
                protocol_version: CURRENT_PROTOCOL_VERSION,
                request_id: None,
                operation_id: OperationId::new("operation"),
                execution_id: ExecutionId::new("execution"),
                payload: CloneRepository { spec: spec.clone() },
            },
            result: Some(CloneExecutionResult::CloneFailed(CloneFailed {
                node: NodeRuntimeIdentity {
                    node_id: NodeId::new("node"),
                    incarnation_id: NodeIncarnationId::new("incarnation"),
                },
                spec,
                failure: CloneFailureCode::Interrupted,
                residual: CloneResidual::Retained {
                    repository_id: RepositoryId::new("repository"),
                    path: NodePath::new("/node/cut"),
                },
            })),
        };
        assert_eq!(
            serde_json::to_value(present(operation)).unwrap(),
            serde_json::json!({
                "operationId": "operation",
                "executionId": "execution",
                "nodeId": "node",
                "repository": "https://example.test/repo.git",
                "branch": "main",
                "state": { "kind": "failed", "reason": "interrupted", "retainedPath": "/node/cut" },
            })
        );
    }
}
