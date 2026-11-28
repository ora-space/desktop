//! Projection of run-local debug refusals and debug store failures onto public errors.
//!
//! The debug store reports a refusal as [`DebugRejection`], either directly or inside the
//! repository error of an entry that met the fence. Each refusal keeps its own public code and
//! class here. Every other failure of the store stays an internal error, like other storage
//! failures.

use super::{BackendError, ErrorClassification};
use ora_application::{ApplicationError, RepositoryError};
use ora_contracts::{
    EmptyErrorParams, PublicError, WorkflowDebugPatchInvalidParams, WorkflowDebugReason as Reason,
    WorkflowDebugTargetRejectedParams,
};
use ora_db::workflow_debug::{DebugPatchReason, DebugRejection};

impl From<DebugRejection> for BackendError {
    /// Projects each refusal onto its public code and class and keeps the refusal as the source.
    fn from(rejection: DebugRejection) -> Self {
        let target_rejected = |detail: Reason| {
            PublicError::WorkflowDebugTargetRejected(WorkflowDebugTargetRejectedParams {
                reason: detail.to_string(),
                detail: Some(Box::new(detail)),
            })
        };
        let (classification, public_error, context) = match &rejection {
            DebugRejection::EmptyRequestId => (
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "workflow debug confirmation requires a request ID",
            ),
            DebugRejection::StalePreview => (
                ErrorClassification::Conflict,
                PublicError::WorkflowDebugStalePreview(EmptyErrorParams {}),
                "workflow debug preview is stale",
            ),
            DebugRejection::OperationActive => (
                ErrorClassification::Conflict,
                PublicError::WorkflowDebugOperationActive(EmptyErrorParams {}),
                "a workflow debug operation is active for the run",
            ),
            DebugRejection::RequestConflict => (
                ErrorClassification::Conflict,
                PublicError::WorkflowDebugRequestConflict(EmptyErrorParams {}),
                "workflow debug request key already names another preview",
            ),
            DebugRejection::MissingInputEvidence => (
                ErrorClassification::InvalidRequest,
                target_rejected(Reason::InputNotRecorded),
                "workflow debug target is rejected",
            ),
            DebugRejection::MissingFact(reason) => (
                ErrorClassification::InvalidRequest,
                target_rejected(reason.clone()),
                "workflow debug target is rejected",
            ),
            DebugRejection::LoopExitCleanupPending => (
                ErrorClassification::InvalidRequest,
                target_rejected(Reason::LoopExitCleanupPending),
                "workflow debug target is rejected",
            ),
            DebugRejection::LoopExitRecoveryRequired => (
                ErrorClassification::InvalidRequest,
                target_rejected(Reason::LoopExitRecoveryRequired),
                "workflow debug target is rejected",
            ),
            DebugRejection::InvalidTarget => (
                ErrorClassification::InvalidRequest,
                target_rejected(Reason::InvalidTarget),
                "workflow debug target is rejected",
            ),
            DebugRejection::InvalidPatch { path, reason } => (
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowDebugPatchInvalid(WorkflowDebugPatchInvalidParams {
                    path: path.clone(),
                    reason: reason.to_string(),
                    detail: match reason {
                        DebugPatchReason::Detail(detail) => Some(Box::new(detail.clone())),
                        DebugPatchReason::Validation(_) => None,
                    },
                }),
                "workflow debug patch is invalid",
            ),
        };
        Self::with_source(classification, public_error, context, rejection)
    }
}

impl BackendError {
    /// Surfaces a refused run-local debug request as its own public error. Any other failure of
    /// the debug store stays internal, like every other storage failure.
    pub fn from_debug_store(error: ora_db::DatabaseError) -> Self {
        match error {
            ora_db::DatabaseError::WorkflowDebug(rejection) => rejection.into(),
            other => Self::internal("workflow debug store operation failed", other),
        }
    }

    /// Does the same for an entry that reports the refusal through its repository error, such as
    /// a snapshot switch that meets the debug fence.
    pub fn from_debug_repository(context: &'static str, error: RepositoryError) -> Self {
        match error.downcast_ref::<ora_db::DatabaseError>() {
            Some(ora_db::DatabaseError::WorkflowDebug(rejection)) => rejection.clone().into(),
            Some(_) | None => Self::internal(context, error),
        }
    }

    /// Does the same for a workflow-run use case that reports the refusal through its application
    /// error, such as failure recovery or a restart that meets the debug fence.
    pub fn from_run_use_case(error: ApplicationError) -> Self {
        match error {
            ApplicationError::WorkflowRunRepository { source } => {
                Self::from_debug_repository("workflow run repository operation failed", source)
            }
            other => other.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BackendError, ErrorClassification};
    use ora_application::{ApplicationError, RepositoryError};
    use ora_contracts::{
        EmptyErrorParams, PublicError, WorkflowDebugPatchInvalidParams,
        WorkflowDebugReason as Reason, WorkflowDebugTargetRejectedParams,
    };
    use ora_db::workflow_debug::DebugRejection;
    use pretty_assertions::assert_eq;

    /// Every refusal keeps its own public code and class whichever way it reaches the backend.
    /// Any other failure of the debug store, wrapped or not, stays internal.
    #[test]
    fn maps_run_local_debug_refusals_to_distinct_public_errors() {
        let cases = [
            (
                DebugRejection::StalePreview,
                ErrorClassification::Conflict,
                PublicError::WorkflowDebugStalePreview(EmptyErrorParams {}),
            ),
            (
                DebugRejection::OperationActive,
                ErrorClassification::Conflict,
                PublicError::WorkflowDebugOperationActive(EmptyErrorParams {}),
            ),
            (
                DebugRejection::EmptyRequestId,
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
            ),
            (
                DebugRejection::RequestConflict,
                ErrorClassification::Conflict,
                PublicError::WorkflowDebugRequestConflict(EmptyErrorParams {}),
            ),
            (
                DebugRejection::MissingInputEvidence,
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowDebugTargetRejected(WorkflowDebugTargetRejectedParams {
                    reason: Reason::InputNotRecorded.to_string(),
                    detail: Some(Box::new(Reason::InputNotRecorded)),
                }),
            ),
            (
                DebugRejection::MissingFact(Reason::VariableUnavailable {
                    variable: "start.text".into(),
                }),
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowDebugTargetRejected(WorkflowDebugTargetRejectedParams {
                    reason: Reason::VariableUnavailable {
                        variable: "start.text".into(),
                    }
                    .to_string(),
                    detail: Some(Box::new(Reason::VariableUnavailable {
                        variable: "start.text".into(),
                    })),
                }),
            ),
            (
                DebugRejection::InvalidPatch {
                    path: "/cases/0/id".to_string(),
                    reason: Reason::FieldNotEditable.into(),
                },
                ErrorClassification::InvalidRequest,
                PublicError::WorkflowDebugPatchInvalid(WorkflowDebugPatchInvalidParams {
                    path: "/cases/0/id".to_string(),
                    reason: Reason::FieldNotEditable.to_string(),
                    detail: Some(Box::new(Reason::FieldNotEditable)),
                }),
            ),
        ];
        for (rejection, classification, public_error) in cases {
            let wrapped =
                || RepositoryError::new(ora_db::DatabaseError::WorkflowDebug(rejection.clone()));
            for error in [
                BackendError::from_debug_store(ora_db::DatabaseError::WorkflowDebug(
                    rejection.clone(),
                )),
                BackendError::from_debug_repository(
                    "failed to switch workflow run snapshot",
                    wrapped(),
                ),
                BackendError::from_run_use_case(ApplicationError::WorkflowRunRepository {
                    source: wrapped(),
                }),
            ] {
                assert_eq!(
                    (error.classification(), error.public_error().clone()),
                    (classification, public_error.clone())
                );
            }
        }

        let internal = (
            ErrorClassification::Internal,
            PublicError::InternalError(EmptyErrorParams {}),
        );
        let storage = || ora_db::DatabaseError::IncompleteWorkflowRunContext;
        for error in [
            BackendError::from_debug_store(storage()),
            BackendError::from_debug_repository(
                "failed to switch",
                RepositoryError::new(storage()),
            ),
            BackendError::from_run_use_case(ApplicationError::WorkflowRunRepository {
                source: RepositoryError::new(storage()),
            }),
        ] {
            assert_eq!(
                (error.classification(), error.public_error().clone()),
                internal.clone()
            );
        }
    }
}
