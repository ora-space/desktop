use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent-import.ts")]
pub struct PrepareAgentImportRequest {
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent-import.ts")]
pub struct PrepareAgentImportResponse {
    pub candidate: AgentImportCandidate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent-import.ts")]
pub struct AgentImportCandidate {
    pub name: String,
    pub description: String,
    pub status: AgentImportCandidateStatus,
    pub existing_agent: Option<AgentImportConflictInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "agent-import.ts")]
pub enum AgentImportCandidateStatus {
    Ready,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent-import.ts")]
pub struct AgentImportConflictInfo {
    pub agent_id: String,
    #[ts(type = "number")]
    pub updated_at: i64,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "agent-import.ts")]
pub enum AgentImportDecision {
    Skip,
    Overwrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent-import.ts")]
pub struct CommitAgentImportRequest {
    pub content: String,
    pub decision: Option<AgentImportDecision>,
    pub expected_agent_id: Option<String>,
    #[ts(type = "number | null")]
    pub expected_updated_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export_to = "agent-import.ts")]
pub enum AgentImportResultStatus {
    Imported,
    Overwritten,
    Skipped,
    StaleConflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "agent-import.ts")]
pub struct CommitAgentImportResponse {
    pub status: AgentImportResultStatus,
    pub agent: Option<crate::Agent>,
}

pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    PrepareAgentImportRequest::export(config)?;
    PrepareAgentImportResponse::export(config)?;
    AgentImportCandidate::export(config)?;
    AgentImportCandidateStatus::export(config)?;
    AgentImportConflictInfo::export(config)?;
    AgentImportDecision::export(config)?;
    CommitAgentImportRequest::export(config)?;
    AgentImportResultStatus::export(config)?;
    CommitAgentImportResponse::export(config)?;
    Ok(())
}
