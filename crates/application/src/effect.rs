use ora_contracts::{
    DesiredEffectDto, EffectConditionDto, EffectConditionImpactDto, EffectConditionRetryDto,
    EffectConsumerRefDto, EffectParametersDto, EffectProtocolRequirementDto, EffectStateDto,
    EffectTargetInclusionDto, EffectTargetPhaseDto, EffectTargetSelectorDto, EffectTargetStatusDto,
    GetEffectStateRequest, GetEffectStateResponse, GetEffectTargetStatusRequest,
    GetEffectTargetStatusResponse, ReplaceEffectStateRequest, ReplaceEffectStateResponse,
    RetryEffectTargetRequest, RetryEffectTargetResponse,
};
use ora_domain::WorkspaceId;
use ora_effect::{
    CapabilityRequirement, ConditionGeneration, ConditionImpact, ConditionOwner, ConditionRetry,
    ConditionSubject, ConsumerIdentity, ConsumerKind, DesiredEffect, DesiredEffectIdentity,
    DesiredState, EffectCondition, EffectKind, EffectRepository, EffectRevisionId, EffectScopeId,
    EffectTargetId, Generation, LocalTimestamp, ReconcileStage, ReplaceDesiredStateOutcome,
    RepositoryError, SkillParameters, TargetInclusion, TargetPhase, TargetSelector,
    ValidatedEffectParameters,
};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Stable application failures for generic Effect APIs.
#[derive(Debug, Error)]
pub enum EffectApplicationError {
    #[error("invalid Effect Desired State")]
    InvalidDesiredState,
    #[error("Effect generation conflict: expected {expected}, current {current}")]
    GenerationConflict { expected: u64, current: u64 },
    #[error("selected Effect revision is unavailable: {revision_id}")]
    RevisionUnavailable { revision_id: String },
    #[error("Effect Scope is retiring")]
    ScopeRetiring,
    #[error("Effect repository operation failed")]
    Repository(#[source] RepositoryError),
}

/// Handles complete Desired State, generic Target status, and explicit retries over an injected port.
pub struct EffectService<Repository> {
    repository: Repository,
}

impl<Repository> EffectService<Repository>
where
    Repository: EffectRepository,
{
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }

    /// Loads a transaction-consistent complete Desired State snapshot.
    pub fn get(
        &self,
        request: GetEffectStateRequest,
    ) -> Result<GetEffectStateResponse, EffectApplicationError> {
        let state = self
            .repository
            .load_desired_state(&workspace_scope(request.workspace_id))
            .map_err(EffectApplicationError::Repository)?;
        Ok(GetEffectStateResponse {
            state: map_state(state),
        })
    }

    /// Replaces the full Desired Effect set using generation compare-and-swap.
    pub fn replace(
        &self,
        request: ReplaceEffectStateRequest,
        updated_at: i64,
    ) -> Result<ReplaceEffectStateResponse, EffectApplicationError> {
        let scope = workspace_scope(request.workspace_id);
        let effects = request
            .effects
            .into_iter()
            .map(map_desired_effect)
            .collect::<Result<Vec<_>, _>>()?;
        let outcome = self
            .repository
            .replace_desired_state(
                &scope,
                Generation::new(request.expected_generation),
                effects,
                LocalTimestamp::from_millis(updated_at),
            )
            .map_err(EffectApplicationError::Repository)?;
        match outcome {
            ReplaceDesiredStateOutcome::Unchanged(state) => Ok(ReplaceEffectStateResponse {
                state: map_state(state),
                changed: false,
            }),
            ReplaceDesiredStateOutcome::Replaced(state) => Ok(ReplaceEffectStateResponse {
                state: map_state(state),
                changed: true,
            }),
            ReplaceDesiredStateOutcome::Conflict {
                expected_generation,
                current_generation,
            } => Err(EffectApplicationError::GenerationConflict {
                expected: expected_generation.value(),
                current: current_generation.value(),
            }),
            ReplaceDesiredStateOutcome::RevisionUnavailable(revision) => {
                Err(EffectApplicationError::RevisionUnavailable {
                    revision_id: revision.to_string(),
                })
            }
            ReplaceDesiredStateOutcome::ScopeRetiring => Err(EffectApplicationError::ScopeRetiring),
        }
    }

    /// Reads persisted Target status without observing or mutating an external Resource.
    pub fn get_target_status(
        &self,
        request: GetEffectTargetStatusRequest,
    ) -> Result<GetEffectTargetStatusResponse, EffectApplicationError> {
        let status = match request {
            GetEffectTargetStatusRequest::Target { target_id } => self
                .repository
                .load_target_status(&EffectTargetId::new(target_id)),
            GetEffectTargetStatusRequest::WorkspaceAgent {
                workspace_id,
                agent_plugin_id,
            } => {
                let consumer = ConsumerIdentity::new(ConsumerKind::agent_plugin(), agent_plugin_id)
                    .map_err(|_| EffectApplicationError::InvalidDesiredState)?;
                self.repository
                    .load_consumer_target_status(&workspace_scope(workspace_id), &consumer)
            }
        }
        .map_err(EffectApplicationError::Repository)?
        .map(|(status, conditions)| map_target_status(status, conditions));
        Ok(GetEffectTargetStatusResponse { status })
    }

    /// Coalesces a user-requested Target wakeup without changing Desired State.
    pub fn retry_target(
        &self,
        request: RetryEffectTargetRequest,
        requested_at: i64,
    ) -> Result<RetryEffectTargetResponse, EffectApplicationError> {
        let requested = self
            .repository
            .request_reconcile(
                &EffectTargetId::new(request.target_id),
                LocalTimestamp::from_millis(requested_at),
            )
            .map_err(EffectApplicationError::Repository)?;
        Ok(RetryEffectTargetResponse { requested })
    }
}

/// Wraps the first-version Workspace identity in the generic Scope identity.
fn workspace_scope(workspace_id: String) -> EffectScopeId {
    EffectScopeId::Workspace(WorkspaceId::new(workspace_id))
}

/// Converts untrusted transport intent into the closed domain representation.
fn map_desired_effect(effect: DesiredEffectDto) -> Result<DesiredEffect, EffectApplicationError> {
    Ok(DesiredEffect {
        identity: DesiredEffectIdentity::new(effect.id),
        revision: EffectRevisionId::new(effect.revision_id),
        parameters: match effect.parameters {
            EffectParametersDto::Skill => ValidatedEffectParameters::Skill(SkillParameters {}),
        },
        audience: map_selector(effect.audience)?,
    })
}

/// Validates every protocol and Consumer identity in a Target selector.
fn map_selector(
    selector: EffectTargetSelectorDto,
) -> Result<TargetSelector, EffectApplicationError> {
    let effect_protocols = selector
        .required_protocols
        .into_iter()
        .map(|requirement| {
            EffectKind::parse(requirement.kind)
                .map(|kind| (kind, requirement.version))
                .map_err(|_| EffectApplicationError::InvalidDesiredState)
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let include = match selector.include {
        EffectTargetInclusionDto::AllEligible => TargetInclusion::AllEligible,
        EffectTargetInclusionDto::Only(consumers) => TargetInclusion::Only(
            consumers
                .into_iter()
                .map(map_consumer)
                .collect::<Result<BTreeSet<_>, _>>()?,
        ),
    };
    let exclude = selector
        .exclude
        .into_iter()
        .map(map_consumer)
        .collect::<Result<BTreeSet<_>, _>>()?;
    Ok(TargetSelector {
        required_capabilities: CapabilityRequirement {
            effect_protocols,
            materialization_contracts: selector
                .required_materialization_contracts
                .into_iter()
                .collect(),
        },
        include,
        exclude,
    })
}

/// Rejects connection-local or malformed Consumer identities at the transport boundary.
fn map_consumer(
    consumer: EffectConsumerRefDto,
) -> Result<ConsumerIdentity, EffectApplicationError> {
    let kind = ConsumerKind::parse(consumer.kind)
        .map_err(|_| EffectApplicationError::InvalidDesiredState)?;
    ConsumerIdentity::new(kind, consumer.stable_key)
        .map_err(|_| EffectApplicationError::InvalidDesiredState)
}

/// Projects one complete Desired State in deterministic identity order.
fn map_state(state: DesiredState) -> EffectStateDto {
    EffectStateDto {
        workspace_id: state.scope.workspace_id().to_string(),
        generation: state.generation.value(),
        effects: state
            .effects
            .into_values()
            .map(map_desired_effect_dto)
            .collect(),
    }
}

/// Projects a closed domain Desired Effect without leaking persistence details.
fn map_desired_effect_dto(effect: DesiredEffect) -> DesiredEffectDto {
    DesiredEffectDto {
        id: effect.identity.to_string(),
        revision_id: effect.revision.to_string(),
        parameters: match effect.parameters {
            ValidatedEffectParameters::Skill(_) => EffectParametersDto::Skill,
        },
        audience: map_selector_dto(effect.audience),
    }
}

/// Projects a normalized Target selector with stable collection ordering.
fn map_selector_dto(selector: TargetSelector) -> EffectTargetSelectorDto {
    EffectTargetSelectorDto {
        required_protocols: selector
            .required_capabilities
            .effect_protocols
            .into_iter()
            .map(|(kind, version)| EffectProtocolRequirementDto {
                kind: kind.to_string(),
                version,
            })
            .collect(),
        required_materialization_contracts: selector
            .required_capabilities
            .materialization_contracts
            .into_iter()
            .collect(),
        include: match selector.include {
            TargetInclusion::AllEligible => EffectTargetInclusionDto::AllEligible,
            TargetInclusion::Only(consumers) => EffectTargetInclusionDto::Only(
                consumers.into_iter().map(map_consumer_dto).collect(),
            ),
        },
        exclude: selector.exclude.into_iter().map(map_consumer_dto).collect(),
    }
}

/// Projects the stable components of a Consumer identity.
fn map_consumer_dto(consumer: ConsumerIdentity) -> EffectConsumerRefDto {
    EffectConsumerRefDto {
        kind: consumer.kind.to_string(),
        stable_key: consumer.stable_key,
    }
}

/// Projects all Target watermarks and structured current Conditions atomically.
fn map_target_status(
    status: ora_effect::TargetStatus,
    conditions: Vec<EffectCondition>,
) -> EffectTargetStatusDto {
    let recovery_operation_id = match status.phase() {
        TargetPhase::RecoveryRequired(operation) => Some(operation.to_string()),
        TargetPhase::Pending
        | TargetPhase::Reconciling(_)
        | TargetPhase::Current
        | TargetPhase::CurrentWithIssues
        | TargetPhase::Retiring => None,
    };
    EffectTargetStatusDto {
        target_id: status.target().to_string(),
        desired_generation: status.progress().desired().value(),
        observed_generation: status.progress().observed().value(),
        applied_generation: status.progress().applied().value(),
        ready_generation: status.progress().ready().value(),
        phase: map_target_phase(status.phase()),
        status_version: status.version().value(),
        recovery_operation_id,
        updated_at: status.updated_at().millis(),
        conditions: conditions.into_iter().map(map_condition).collect(),
    }
}

/// Maps the generic Target phase without embedding a Condition reason in it.
fn map_target_phase(phase: &TargetPhase) -> EffectTargetPhaseDto {
    match phase {
        TargetPhase::Pending => EffectTargetPhaseDto::Pending,
        TargetPhase::Reconciling(ReconcileStage::Planning) => EffectTargetPhaseDto::Planning,
        TargetPhase::Reconciling(ReconcileStage::Coordinating) => {
            EffectTargetPhaseDto::Coordinating
        }
        TargetPhase::Reconciling(ReconcileStage::Applying) => EffectTargetPhaseDto::Applying,
        TargetPhase::Reconciling(ReconcileStage::Verifying) => EffectTargetPhaseDto::Verifying,
        TargetPhase::Reconciling(ReconcileStage::Activating) => EffectTargetPhaseDto::Activating,
        TargetPhase::Current => EffectTargetPhaseDto::Current,
        TargetPhase::CurrentWithIssues => EffectTargetPhaseDto::CurrentWithIssues,
        TargetPhase::Retiring => EffectTargetPhaseDto::Retiring,
        TargetPhase::RecoveryRequired(_) => EffectTargetPhaseDto::RecoveryRequired,
    }
}

/// Flattens typed Condition identities into stable transport fields.
fn map_condition(condition: EffectCondition) -> EffectConditionDto {
    let (owner_kind, owner_id) = match condition.owner {
        ConditionOwner::Target(target) => ("target", target.to_string()),
        ConditionOwner::Resource(resource) => ("resource", resource.to_string()),
    };
    let (subject_kind, subject_id) = match condition.subject {
        ConditionSubject::Consumer(consumer) => ("consumer", consumer.to_string()),
        ConditionSubject::Target(target) => ("target", target.to_string()),
        ConditionSubject::DesiredEffect(desired) => ("desired_effect", desired.to_string()),
        ConditionSubject::Resource(resource) => ("resource", resource.to_string()),
        ConditionSubject::ManagedItem(managed) => ("managed_item", managed.to_string()),
        ConditionSubject::Operation(operation) => ("operation", operation.to_string()),
        ConditionSubject::Artifact(artifact) => ("artifact", artifact.to_string()),
    };
    EffectConditionDto {
        id: condition.identity.to_string(),
        owner_kind: owner_kind.to_string(),
        owner_id,
        subject_kind: subject_kind.to_string(),
        subject_id,
        code: condition.code.as_str().to_string(),
        impact: match condition.impact {
            ConditionImpact::Blocking => EffectConditionImpactDto::Blocking,
            ConditionImpact::NonBlocking => EffectConditionImpactDto::NonBlocking,
        },
        retry: match condition.retry {
            ConditionRetry::OnChange => EffectConditionRetryDto::OnChange,
            ConditionRetry::Backoff(_) => EffectConditionRetryDto::Backoff,
            ConditionRetry::Manual => EffectConditionRetryDto::Manual,
        },
        generation: match condition.generation {
            ConditionGeneration::Unscoped => None,
            ConditionGeneration::At(generation) => Some(generation.value()),
        },
        message: condition.safe_details.message,
        first_observed_at: condition.first_observed_at.millis(),
        last_observed_at: condition.last_observed_at.millis(),
    }
}
