use serde::{Deserialize, Serialize};
use ts_rs::{Config, ExportError, TS};

/// Catalog owner of one desired Skill selection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "effect.ts")]
pub enum EffectSourceKind {
    Local,
    Plugin,
}

/// Exact catalog-backed Skill state supplied as part of a complete desired replacement.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct DesiredSkillStateDto {
    pub source_kind: EffectSourceKind,
    pub namespace: String,
    pub name: String,
    pub version: String,
    pub skill_md_digest: String,
}

/// Public projection of one complete Workspace desired generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct WorkspaceEffectDto {
    pub workspace_id: String,
    #[ts(type = "number")]
    pub generation: u64,
    pub skills: Vec<DesiredSkillStateDto>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct GetWorkspaceEffectRequest {
    pub workspace_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct GetWorkspaceEffectResponse {
    pub effect: WorkspaceEffectDto,
}

/// Full replacement request using optimistic generation compare-and-swap.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct ReplaceWorkspaceEffectRequest {
    pub workspace_id: String,
    #[ts(type = "number")]
    pub expected_generation: u64,
    pub skills: Vec<DesiredSkillStateDto>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct ReplaceWorkspaceEffectResponse {
    pub effect: WorkspaceEffectDto,
    pub changed: bool,
}

/// Stable reconcile phases without embedding a blocking reason in the phase itself.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "effect.ts")]
pub enum EffectSurfacePhase {
    Pending,
    WaitingForIdle,
    Quiescing,
    Applying,
    Resuming,
    Current,
    Degraded,
    Retiring,
    RecoveryRequired,
}

/// Retry behavior derived from a condition's stable reason.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "effect.ts")]
pub enum EffectRetryPolicy {
    OnChange,
    Backoff,
    Manual,
}

/// Transport projection of a structured current Effect condition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct EffectConditionDto {
    pub subject_kind: String,
    pub subject_id: String,
    pub reason: String,
    pub message: String,
    pub first_occurred_at: i64,
    pub last_occurred_at: i64,
    #[ts(type = "number")]
    pub failed_generation: u64,
    pub retry_policy: EffectRetryPolicy,
}

/// Transaction-consistent persisted surface status snapshot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct EffectSurfaceStatusDto {
    pub workspace_id: String,
    pub surface_key: String,
    #[ts(type = "number")]
    pub desired_generation: u64,
    #[ts(type = "number")]
    pub observed_generation: u64,
    #[ts(type = "number")]
    pub applied_generation: u64,
    pub phase: EffectSurfacePhase,
    #[ts(type = "number")]
    pub revision: u64,
    pub updated_at: i64,
    pub conditions: Vec<EffectConditionDto>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct GetEffectSurfaceStatusRequest {
    pub workspace_id: String,
    pub surface_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct GetEffectSurfaceStatusResponse {
    pub status: Option<EffectSurfaceStatusDto>,
}

/// Explicit retry only wakes reconciliation; it never mutates Desired or bypasses conflicts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct RetryEffectSurfaceRequest {
    pub workspace_id: String,
    pub surface_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "effect.ts")]
pub struct RetryEffectSurfaceResponse {
    pub requested: bool,
}

/// Exports Effect contract bindings beside the other transport-neutral DTO families.
pub(crate) fn export(config: &Config) -> Result<(), ExportError> {
    EffectSourceKind::export_all(config)?;
    DesiredSkillStateDto::export_all(config)?;
    WorkspaceEffectDto::export_all(config)?;
    GetWorkspaceEffectRequest::export_all(config)?;
    GetWorkspaceEffectResponse::export_all(config)?;
    ReplaceWorkspaceEffectRequest::export_all(config)?;
    ReplaceWorkspaceEffectResponse::export_all(config)?;
    EffectSurfacePhase::export_all(config)?;
    EffectRetryPolicy::export_all(config)?;
    EffectConditionDto::export_all(config)?;
    EffectSurfaceStatusDto::export_all(config)?;
    GetEffectSurfaceStatusRequest::export_all(config)?;
    GetEffectSurfaceStatusResponse::export_all(config)?;
    RetryEffectSurfaceRequest::export_all(config)?;
    RetryEffectSurfaceResponse::export_all(config)?;
    Ok(())
}
