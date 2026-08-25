use ora_contracts::{
    DesiredSkillStateDto, EffectConditionDto, EffectRetryPolicy, EffectSourceKind,
    EffectSurfacePhase, EffectSurfaceStatusDto, GetEffectSurfaceStatusRequest,
    GetEffectSurfaceStatusResponse, GetWorkspaceEffectRequest, GetWorkspaceEffectResponse,
    ReplaceWorkspaceEffectRequest, ReplaceWorkspaceEffectResponse, RetryEffectSurfaceRequest,
    RetryEffectSurfaceResponse, WorkspaceEffectDto,
};
use ora_domain::{Namespace, WorkspaceId};
use ora_effect::{
    ConditionReason, ConditionSubject, DesiredSkillState, Digest, EffectRepository, Generation,
    ReplaceEffectOutcome, RepositoryError, RetryPolicy, SkillName, SkillSource, SkillState,
    SourceKind, SourceVersion, SurfaceKey, SurfacePhase, SurfaceStatus, WorkspaceEffect,
    WorkspaceEffectSpec,
};
use thiserror::Error;

/// Stable application failures for Workspace Effect APIs.
#[derive(Debug, Error)]
pub enum EffectApplicationError {
    #[error("invalid Workspace Effect specification")]
    InvalidSpec,
    #[error("Workspace Effect generation conflict: expected {expected}, current {current}")]
    GenerationConflict { expected: u64, current: u64 },
    #[error("selected Effect source is unavailable: {source_kind}/{namespace}/{name}")]
    SourceUnavailable {
        source_kind: &'static str,
        namespace: String,
        name: String,
    },
    #[error("Effect repository operation failed")]
    Repository(#[source] RepositoryError),
}

/// Handles desired reads/replacements, status reads, and explicit retries over an injected port.
pub struct WorkspaceEffectService<Repository> {
    repository: Repository,
}

impl<Repository> WorkspaceEffectService<Repository>
where
    Repository: EffectRepository,
{
    pub fn new(repository: Repository) -> Self {
        Self { repository }
    }

    /// Loads a transaction-consistent complete Desired snapshot.
    pub fn get(
        &self,
        request: GetWorkspaceEffectRequest,
    ) -> Result<GetWorkspaceEffectResponse, EffectApplicationError> {
        let effect = self
            .repository
            .load_workspace_effect(&WorkspaceId::new(request.workspace_id))
            .map_err(EffectApplicationError::Repository)?;
        Ok(GetWorkspaceEffectResponse {
            effect: map_effect(effect),
        })
    }

    /// Replaces the full desired set with generation CAS and exact normalized no-op behavior.
    pub fn replace(
        &self,
        request: ReplaceWorkspaceEffectRequest,
        updated_at: i64,
    ) -> Result<ReplaceWorkspaceEffectResponse, EffectApplicationError> {
        let workspace_id = WorkspaceId::new(request.workspace_id);
        let spec = map_spec(request.skills)?;
        let outcome = self
            .repository
            .replace_workspace_effect(
                &workspace_id,
                Generation::new(request.expected_generation),
                spec,
                updated_at,
            )
            .map_err(EffectApplicationError::Repository)?;
        match outcome {
            ReplaceEffectOutcome::Unchanged(effect) => Ok(ReplaceWorkspaceEffectResponse {
                effect: map_effect(effect),
                changed: false,
            }),
            ReplaceEffectOutcome::Replaced(effect) => Ok(ReplaceWorkspaceEffectResponse {
                effect: map_effect(effect),
                changed: true,
            }),
            ReplaceEffectOutcome::Conflict {
                expected_generation,
                current_generation,
            } => Err(EffectApplicationError::GenerationConflict {
                expected: expected_generation.value(),
                current: current_generation.value(),
            }),
            ReplaceEffectOutcome::SourceUnavailable { selection_key } => {
                Err(EffectApplicationError::SourceUnavailable {
                    source_kind: source_kind_value(selection_key.source_kind),
                    namespace: selection_key.namespace.to_string(),
                    name: selection_key.name.to_string(),
                })
            }
        }
    }

    /// Reads persisted status without running a filesystem scan or advancing Desired generation.
    pub fn get_status(
        &self,
        request: GetEffectSurfaceStatusRequest,
    ) -> Result<GetEffectSurfaceStatusResponse, EffectApplicationError> {
        let workspace_id = WorkspaceId::new(request.workspace_id);
        let surface_key = SurfaceKey::new(request.surface_key);
        let status = self
            .repository
            .load_surface_status(&workspace_id, &surface_key)
            .map_err(EffectApplicationError::Repository)?
            .map(map_status);
        Ok(GetEffectSurfaceStatusResponse { status })
    }

    /// Coalesces an explicit retry wakeup without changing Desired or bypassing conditions.
    pub fn retry(
        &self,
        request: RetryEffectSurfaceRequest,
        requested_at: i64,
    ) -> Result<RetryEffectSurfaceResponse, EffectApplicationError> {
        let requested = self
            .repository
            .retry_surface(
                &WorkspaceId::new(request.workspace_id),
                &SurfaceKey::new(request.surface_key),
                requested_at,
            )
            .map_err(EffectApplicationError::Repository)?;
        Ok(RetryEffectSurfaceResponse { requested })
    }
}

/// Converts untrusted contract fields into a normalized complete Effect specification.
fn map_spec(
    skills: Vec<DesiredSkillStateDto>,
) -> Result<WorkspaceEffectSpec, EffectApplicationError> {
    let desired = skills
        .into_iter()
        .map(|skill| {
            let name =
                SkillName::parse(skill.name).map_err(|_| EffectApplicationError::InvalidSpec)?;
            let namespace =
                Namespace::new(skill.namespace).map_err(|_| EffectApplicationError::InvalidSpec)?;
            let version = SourceVersion::parse(skill.version)
                .map_err(|_| EffectApplicationError::InvalidSpec)?;
            let source = match skill.source_kind {
                EffectSourceKind::Local => SkillSource::Local { namespace, version },
                EffectSourceKind::Plugin => SkillSource::Plugin { namespace, version },
            };
            DesiredSkillState::try_new(SkillState {
                name,
                skill_md_digest: Digest::parse(skill.skill_md_digest)
                    .map_err(|_| EffectApplicationError::InvalidSpec)?,
                source,
            })
            .map_err(|_| EffectApplicationError::InvalidSpec)
        })
        .collect::<Result<Vec<_>, _>>()?;
    WorkspaceEffectSpec::normalized(desired).map_err(|_| EffectApplicationError::InvalidSpec)
}

/// Projects one domain Desired snapshot into stable contract ordering.
fn map_effect(effect: WorkspaceEffect) -> WorkspaceEffectDto {
    WorkspaceEffectDto {
        workspace_id: effect.workspace_id.to_string(),
        generation: effect.generation.value(),
        skills: effect
            .spec
            .skills
            .into_values()
            .map(|desired| {
                let state = desired.state();
                let (source_kind, namespace, version) = match &state.source {
                    SkillSource::Local { namespace, version } => {
                        (EffectSourceKind::Local, namespace, version)
                    }
                    SkillSource::Plugin { namespace, version } => {
                        (EffectSourceKind::Plugin, namespace, version)
                    }
                    SkillSource::Preserved { .. } => unreachable!(
                        "DesiredSkillState prevents preserved state from reaching contracts"
                    ),
                };
                DesiredSkillStateDto {
                    source_kind,
                    namespace: namespace.to_string(),
                    name: state.name.to_string(),
                    version: version.to_string(),
                    skill_md_digest: state.skill_md_digest.to_string(),
                }
            })
            .collect(),
    }
}

/// Projects persisted status and its structured current conditions.
fn map_status(status: SurfaceStatus) -> EffectSurfaceStatusDto {
    EffectSurfaceStatusDto {
        workspace_id: status.workspace_id.to_string(),
        surface_key: status.surface_key.to_string(),
        desired_generation: status.desired_generation.value(),
        observed_generation: status.observed_generation.value(),
        applied_generation: status.applied_generation.value(),
        phase: map_phase(status.phase),
        revision: status.revision,
        updated_at: status.updated_at,
        conditions: status.conditions.into_iter().map(map_condition).collect(),
    }
}

/// Flattens the tagged condition subject into stable kind and identifier contract fields.
fn map_condition(condition: ora_effect::Condition) -> EffectConditionDto {
    let (subject_kind, subject_id) = match condition.subject {
        ConditionSubject::DesiredSkill { selection_key } => (
            "desired_skill".to_string(),
            format!(
                "{}/{}/{}",
                source_kind_value(selection_key.source_kind),
                selection_key.namespace,
                selection_key.name
            ),
        ),
        ConditionSubject::ManagedSkill { managed_identity } => {
            ("managed_skill".to_string(), managed_identity.to_string())
        }
        ConditionSubject::Surface { surface_key } => {
            ("surface".to_string(), surface_key.to_string())
        }
        ConditionSubject::Consumer { consumer_id } => {
            ("consumer".to_string(), consumer_id.to_string())
        }
    };
    EffectConditionDto {
        subject_kind,
        subject_id,
        reason: condition_reason_value(condition.reason).to_string(),
        message: condition.message,
        first_occurred_at: condition.first_occurred_at,
        last_occurred_at: condition.last_occurred_at,
        failed_generation: condition.failed_generation.value(),
        retry_policy: match condition.retry_policy {
            RetryPolicy::OnChange => EffectRetryPolicy::OnChange,
            RetryPolicy::Backoff => EffectRetryPolicy::Backoff,
            RetryPolicy::Manual => EffectRetryPolicy::Manual,
        },
    }
}

fn source_kind_value(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Local => "local",
        SourceKind::Plugin => "plugin",
    }
}

fn map_phase(phase: SurfacePhase) -> EffectSurfacePhase {
    match phase {
        SurfacePhase::Pending => EffectSurfacePhase::Pending,
        SurfacePhase::WaitingForIdle => EffectSurfacePhase::WaitingForIdle,
        SurfacePhase::Quiescing => EffectSurfacePhase::Quiescing,
        SurfacePhase::Applying => EffectSurfacePhase::Applying,
        SurfacePhase::Resuming => EffectSurfacePhase::Resuming,
        SurfacePhase::Current => EffectSurfacePhase::Current,
        SurfacePhase::Degraded => EffectSurfacePhase::Degraded,
        SurfacePhase::Retiring => EffectSurfacePhase::Retiring,
        SurfacePhase::RecoveryRequired => EffectSurfacePhase::RecoveryRequired,
    }
}

fn condition_reason_value(reason: ConditionReason) -> &'static str {
    match reason {
        ConditionReason::NoConsumers => "no_consumers",
        ConditionReason::IncompatibleSurfaceDeclarations => "incompatible_surface_declarations",
        ConditionReason::DesiredCollision => "desired_collision",
        ConditionReason::PreservedConflict => "preserved_conflict",
        ConditionReason::OwnershipConflict => "ownership_conflict",
        ConditionReason::DriftConflict => "drift_conflict",
        ConditionReason::SourceUnavailable => "source_unavailable",
        ConditionReason::PathUnsafe => "path_unsafe",
        ConditionReason::ScanFailed => "scan_failed",
        ConditionReason::WaitingForIdle => "waiting_for_idle",
        ConditionReason::ConsumerResumeFailed => "consumer_resume_failed",
        ConditionReason::MaterializationFailed => "materialization_failed",
        ConditionReason::TransientIo => "transient_io",
        ConditionReason::RecoveryRequired => "recovery_required",
    }
}
