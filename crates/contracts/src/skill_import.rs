use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Names the physical source of one import session.
///
/// Adapters materialize the raw source into OS temporary storage before handing paths here;
/// the paths are never treated as untrusted names for storage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub enum SkillImportSource {
    /// A local folder tree whose absolute path is read by the backend.
    Folder { path: String },
    /// A local archive file plus its original file name used for extension detection.
    Archive {
        path: String,
        #[serde(rename = "fileName")]
        file_name: String,
    },
}

/// Requests the two-phase preparation of one skill import source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct PrepareSkillImportRequest {
    pub source: SkillImportSource,
}

/// Returns the prepared session with its full preview after successful preparation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct PrepareSkillImportResponse {
    pub session: SkillImportSession,
}

/// Identifies one prepared import session by its opaque identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct GetSkillImportSessionRequest {
    pub session_id: String,
}

/// Returns one import session with its current status and progress.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct GetSkillImportSessionResponse {
    pub session: SkillImportSession,
}

/// Submits explicit decisions for every conflict candidate and freezes the commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct CommitSkillImportRequest {
    pub session_id: String,
    pub decisions: Vec<SkillImportConflictDecision>,
}

/// Confirms the commit was accepted and is running as a background task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct CommitSkillImportResponse {
    pub session_id: String,
    pub status: SkillImportSessionStatus,
    pub progress: SkillImportProgress,
}

/// Cancels a prepared import session before any commit has been accepted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct CancelSkillImportRequest {
    pub session_id: String,
}

/// Reports whether a prepared session was cancelled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct CancelSkillImportResponse {
    pub session_id: String,
    pub cancelled: bool,
}

/// One live import session projection shared by every transport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct SkillImportSession {
    pub session_id: String,
    pub status: SkillImportSessionStatus,
    pub created_at: i64,
    pub candidates: Vec<SkillImportCandidate>,
    pub progress: SkillImportProgress,
}

/// Tracks the lifecycle phase of one import session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "skill-import.ts")]
pub enum SkillImportSessionStatus {
    Prepared,
    Committing,
    Completed,
    Cancelled,
}

/// One previewed skill candidate discovered in the source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct SkillImportCandidate {
    pub candidate_id: String,
    pub name: String,
    pub description: String,
    /// Safe, validated relative path of the candidate's `SKILL.md` within the source.
    pub source_path: String,
    pub file_count: usize,
    pub total_size: u64,
    pub status: SkillImportCandidateStatus,
    /// Stable machine-readable reason for `invalid` candidates.
    pub error_code: Option<String>,
    /// Present only for `conflict` candidates.
    pub existing_skill: Option<SkillConflictInfo>,
}

/// Classifies one candidate's readiness for commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "skill-import.ts")]
pub enum SkillImportCandidateStatus {
    Ready,
    Conflict,
    Invalid,
}

/// Describes the existing visible skill a `conflict` candidate would overwrite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct SkillConflictInfo {
    pub skill_id: String,
    pub updated_at: i64,
    pub description: String,
}

/// Carries the explicit decision for one conflict candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct SkillImportConflictDecision {
    pub candidate_id: String,
    pub decision: SkillImportDecision,
}

/// The user-selected handling for one conflict candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "skill-import.ts")]
pub enum SkillImportDecision {
    Skip,
    Overwrite,
}

/// One per-candidate outcome after the commit task processes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct SkillImportResult {
    pub candidate_id: String,
    pub name: String,
    pub status: SkillImportResultStatus,
    /// Stable machine-readable failure reason for `failed` results.
    pub error_code: Option<String>,
}

/// The final outcome of one committed candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export_to = "skill-import.ts")]
pub enum SkillImportResultStatus {
    Imported,
    Overwritten,
    Skipped,
    Failed,
    StaleConflict,
}

/// Reports the running progress and completed per-item results of a commit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "skill-import.ts")]
pub struct SkillImportProgress {
    pub total: usize,
    pub processed: usize,
    pub results: Vec<SkillImportResult>,
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    SkillImportSource::export(config)?;
    PrepareSkillImportRequest::export(config)?;
    PrepareSkillImportResponse::export(config)?;
    GetSkillImportSessionRequest::export(config)?;
    GetSkillImportSessionResponse::export(config)?;
    CommitSkillImportRequest::export(config)?;
    CommitSkillImportResponse::export(config)?;
    CancelSkillImportRequest::export(config)?;
    CancelSkillImportResponse::export(config)?;
    SkillImportSession::export(config)?;
    SkillImportSessionStatus::export(config)?;
    SkillImportCandidate::export(config)?;
    SkillImportCandidateStatus::export(config)?;
    SkillConflictInfo::export(config)?;
    SkillImportConflictDecision::export(config)?;
    SkillImportDecision::export(config)?;
    SkillImportResult::export(config)?;
    SkillImportResultStatus::export(config)?;
    SkillImportProgress::export(config)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    /// Verifies source kinds serialize with an explicit discriminant tag.
    #[test]
    fn serializes_import_sources() {
        assert_eq!(
            serde_json::to_value(SkillImportSource::Folder {
                path: "/tmp/source".to_string(),
            })
            .unwrap(),
            json!({ "kind": "folder", "path": "/tmp/source" })
        );
        assert_eq!(
            serde_json::to_value(SkillImportSource::Archive {
                path: "/tmp/source.zip".to_string(),
                file_name: "source.zip".to_string(),
            })
            .unwrap(),
            json!({ "kind": "archive", "path": "/tmp/source.zip", "fileName": "source.zip" })
        );
    }

    /// Verifies session projections round-trip their lifecycle and candidate fields.
    #[test]
    fn serializes_session_projection() {
        let session = SkillImportSession {
            session_id: "session-1".to_string(),
            status: SkillImportSessionStatus::Prepared,
            created_at: 1_700_000_000_000,
            candidates: vec![SkillImportCandidate {
                candidate_id: "candidate-1".to_string(),
                name: "review".to_string(),
                description: "Reviews changes".to_string(),
                source_path: "skills/review/SKILL.md".to_string(),
                file_count: 3,
                total_size: 2048,
                status: SkillImportCandidateStatus::Conflict,
                error_code: None,
                existing_skill: Some(SkillConflictInfo {
                    skill_id: "skill-1".to_string(),
                    updated_at: 1_700_000_000_500,
                    description: "Existing review".to_string(),
                }),
            }],
            progress: SkillImportProgress::default(),
        };

        assert_eq!(
            serde_json::to_value(session).unwrap(),
            json!({
                "sessionId": "session-1",
                "status": "prepared",
                "createdAt": 1_700_000_000_000_i64,
                "candidates": [{
                    "candidateId": "candidate-1",
                    "name": "review",
                    "description": "Reviews changes",
                    "sourcePath": "skills/review/SKILL.md",
                    "fileCount": 3,
                    "totalSize": 2048,
                    "status": "conflict",
                    "errorCode": null,
                    "existingSkill": {
                        "skillId": "skill-1",
                        "updatedAt": 1_700_000_000_500_i64,
                        "description": "Existing review",
                    },
                }],
                "progress": { "total": 0, "processed": 0, "results": [] },
            })
        );
    }
}
