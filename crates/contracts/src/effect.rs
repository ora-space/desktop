use serde::{Deserialize, Serialize};
use ts_rs::{Config, ExportError, TS};

/// One stable Consumer identity used by an Effect Target selector.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct EffectConsumerRefDto {
    pub kind: String,
    pub stable_key: String,
}

/// One exact Effect protocol version required from a Consumer Revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct EffectProtocolRequirementDto {
    pub kind: String,
    pub version: u32,
}

/// Explicit inclusion mode for a Desired Effect's Target audience.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "kind", content = "consumers", rename_all = "snake_case")]
#[ts(export_to = "effect.ts")]
pub enum EffectTargetInclusionDto {
    AllEligible,
    Only(Vec<EffectConsumerRefDto>),
}

/// Complete capability and identity selector for one Desired Effect.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct EffectTargetSelectorDto {
    pub required_protocols: Vec<EffectProtocolRequirementDto>,
    pub required_materialization_contracts: Vec<String>,
    pub include: EffectTargetInclusionDto,
    pub exclude: Vec<EffectConsumerRefDto>,
}

/// Closed transport representation of validated kind-specific Effect parameters.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export_to = "effect.ts")]
pub enum EffectParametersDto {
    Skill,
}

/// One stable item of intent selecting an exact immutable Effect Revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct DesiredEffectDto {
    pub id: String,
    pub revision_id: String,
    pub parameters: EffectParametersDto,
    pub audience: EffectTargetSelectorDto,
}

/// Public projection of one complete Workspace Effect Scope generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct EffectStateDto {
    pub workspace_id: String,
    #[ts(type = "number")]
    pub generation: u64,
    pub effects: Vec<DesiredEffectDto>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct GetEffectStateRequest {
    pub workspace_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct GetEffectStateResponse {
    pub state: EffectStateDto,
}

/// Complete replacement request using optimistic generation compare-and-swap.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct ReplaceEffectStateRequest {
    pub workspace_id: String,
    #[ts(type = "number")]
    pub expected_generation: u64,
    pub effects: Vec<DesiredEffectDto>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct ReplaceEffectStateResponse {
    pub state: EffectStateDto,
    pub changed: bool,
}

/// Generic Target convergence position without Consumer-specific runtime phases.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "effect.ts")]
pub enum EffectTargetPhaseDto {
    Pending,
    Planning,
    Coordinating,
    Applying,
    Verifying,
    Activating,
    Current,
    CurrentWithIssues,
    Retiring,
    RecoveryRequired,
}

/// Whether a current Condition blocks its owner's watermark progression.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "effect.ts")]
pub enum EffectConditionImpactDto {
    Blocking,
    NonBlocking,
}

/// Stable retry classification without exposing internal scheduler parameters.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "effect.ts")]
pub enum EffectConditionRetryDto {
    OnChange,
    Backoff,
    Manual,
}

/// Transport projection of one current structured Effect Condition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct EffectConditionDto {
    pub id: String,
    pub owner_kind: String,
    pub owner_id: String,
    pub subject_kind: String,
    pub subject_id: String,
    pub code: String,
    pub impact: EffectConditionImpactDto,
    pub retry: EffectConditionRetryDto,
    pub generation: Option<u64>,
    pub message: String,
    pub first_observed_at: i64,
    pub last_observed_at: i64,
}

/// Transaction-consistent persisted Target status and its current Conditions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct EffectTargetStatusDto {
    pub target_id: String,
    #[ts(type = "number")]
    pub desired_generation: u64,
    #[ts(type = "number")]
    pub observed_generation: u64,
    #[ts(type = "number")]
    pub applied_generation: u64,
    #[ts(type = "number")]
    pub ready_generation: u64,
    pub phase: EffectTargetPhaseDto,
    #[ts(type = "number")]
    pub status_version: u64,
    pub recovery_operation_id: Option<String>,
    pub updated_at: i64,
    pub conditions: Vec<EffectConditionDto>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(
    tag = "selector",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[ts(export_to = "effect.ts")]
pub enum GetEffectTargetStatusRequest {
    Target {
        target_id: String,
    },
    WorkspaceAgent {
        workspace_id: String,
        agent_plugin_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct GetEffectTargetStatusResponse {
    pub status: Option<EffectTargetStatusDto>,
}

/// Explicit retry only wakes Target reconciliation; it never mutates Desired State.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct RetryEffectTargetRequest {
    pub target_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct RetryEffectTargetResponse {
    pub requested: bool,
}

/// Exports Effect contract bindings beside the other transport-neutral DTO families.
pub(crate) fn export(config: &Config) -> Result<(), ExportError> {
    EffectConsumerRefDto::export_all(config)?;
    EffectProtocolRequirementDto::export_all(config)?;
    EffectTargetInclusionDto::export_all(config)?;
    EffectTargetSelectorDto::export_all(config)?;
    EffectParametersDto::export_all(config)?;
    DesiredEffectDto::export_all(config)?;
    EffectStateDto::export_all(config)?;
    GetEffectStateRequest::export_all(config)?;
    GetEffectStateResponse::export_all(config)?;
    ReplaceEffectStateRequest::export_all(config)?;
    ReplaceEffectStateResponse::export_all(config)?;
    EffectTargetPhaseDto::export_all(config)?;
    EffectConditionImpactDto::export_all(config)?;
    EffectConditionRetryDto::export_all(config)?;
    EffectConditionDto::export_all(config)?;
    EffectTargetStatusDto::export_all(config)?;
    GetEffectTargetStatusRequest::export_all(config)?;
    GetEffectTargetStatusResponse::export_all(config)?;
    RetryEffectTargetRequest::export_all(config)?;
    RetryEffectTargetResponse::export_all(config)?;
    Ok(())
}
