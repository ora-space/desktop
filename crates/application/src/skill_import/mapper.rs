use super::ports::{CandidateDecision, CandidateResultStatus, CandidateStatus, ImportSessionState};
use ora_contracts::{
    SkillConflictInfo, SkillImportCandidate, SkillImportCandidateStatus, SkillImportProgress,
    SkillImportResult, SkillImportResultStatus, SkillImportSession,
};
/// Projects one session state onto its public transport contract.
pub(crate) fn project_session(state: &ImportSessionState) -> SkillImportSession {
    SkillImportSession {
        session_id: state.id.clone(),
        status: state.status.clone(),
        created_at: state.created_at,
        candidates: state.candidates.iter().map(project_candidate).collect(),
        progress: project_progress(state),
    }
}

/// Projects one candidate onto its public preview contract.
fn project_candidate(candidate: &super::ports::ImportCandidate) -> SkillImportCandidate {
    let status = match candidate.status {
        CandidateStatus::Ready => SkillImportCandidateStatus::Ready,
        CandidateStatus::Conflict => SkillImportCandidateStatus::Conflict,
        CandidateStatus::Invalid => SkillImportCandidateStatus::Invalid,
    };
    let existing_skill = candidate
        .existing_skill
        .as_ref()
        .map(|info| SkillConflictInfo {
            skill_id: info.skill_id.to_string(),
            updated_at: info.updated_at,
            description: info.description.clone(),
        });
    SkillImportCandidate {
        candidate_id: candidate.candidate_id.clone(),
        name: candidate.name.clone(),
        description: candidate.description.clone(),
        source_path: candidate.source_path.to_string(),
        file_count: candidate.boundary.file_count(),
        total_size: candidate.boundary.total_size(),
        status,
        error_code: candidate.error_code.clone(),
        existing_skill,
    }
}

/// Projects the current progress and completed results onto the public contract.
pub(crate) fn project_progress(state: &ImportSessionState) -> SkillImportProgress {
    SkillImportProgress {
        total: state.total,
        processed: state.processed,
        results: state.results.iter().map(project_result).collect(),
    }
}

/// Projects one finished result onto its public contract.
fn project_result(result: &super::ports::ImportResult) -> SkillImportResult {
    let status = match result.status {
        CandidateResultStatus::Imported => SkillImportResultStatus::Imported,
        CandidateResultStatus::Overwritten => SkillImportResultStatus::Overwritten,
        CandidateResultStatus::Skipped => SkillImportResultStatus::Skipped,
        CandidateResultStatus::Failed => SkillImportResultStatus::Failed,
        CandidateResultStatus::StaleConflict => SkillImportResultStatus::StaleConflict,
    };
    SkillImportResult {
        candidate_id: result.candidate_id.clone(),
        name: result.name.clone(),
        status,
        error_code: result.error_code.clone(),
    }
}

/// Converts one contract decision into its internal representation.
pub(crate) fn to_internal_decision(
    decision: &ora_contracts::SkillImportDecision,
) -> CandidateDecision {
    match decision {
        ora_contracts::SkillImportDecision::Skip => CandidateDecision::Skip,
        ora_contracts::SkillImportDecision::Overwrite => CandidateDecision::Overwrite,
    }
}
