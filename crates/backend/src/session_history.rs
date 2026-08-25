use ora_application::SessionRepository;
use ora_db::{RepositoryPool, SqliteSessionRepository, SqliteWorkspaceRepository};
use ora_domain::{ProjectId, Session, SessionId, WorkspaceId};
use ora_history::remove_session_history;
use ora_logging::ora_warn;
use std::path::Path;

/// Deletes the history files of sessions whose records were removed.
///
/// Ora's soft delete is what a user experiences as deletion, so the conversation
/// it covers goes with it. Removal is best effort: the rows are already gone by
/// the time this runs, and a file left behind is unreachable, while failing here
/// would leave the user with something they cannot delete.
pub(crate) fn remove_session_histories(
    sessions_root: &Path,
    session_ids: impl IntoIterator<Item = SessionId>,
) {
    for session_id in session_ids {
        if let Err(error) = remove_session_history(sessions_root, session_id.as_ref()) {
            ora_warn!(
                session_id = %session_id,
                error = %error,
                "failed to remove session history file",
            );
        }
    }
}

/// Collects the sessions a workspace cascade will remove, before it removes them.
///
/// The lookup has to happen first: once the rows are soft-deleted, nothing links
/// the files back to the task that owned them.
pub(crate) fn session_ids_for_workspace(
    pool: &RepositoryPool,
    workspace_id: &WorkspaceId,
) -> Vec<SessionId> {
    visible_sessions(pool)
        .into_iter()
        .filter(|session| session.workspace_id == *workspace_id)
        .map(|session| session.id)
        .collect()
}

/// Collects the sessions a project cascade will remove, across all of its workspaces.
pub(crate) fn session_ids_for_project(
    pool: &RepositoryPool,
    project_id: &ProjectId,
) -> Vec<SessionId> {
    let workspace_ids: Vec<WorkspaceId> =
        match SqliteWorkspaceRepository::new(pool.clone()).list_workspaces(project_id) {
            Ok(workspaces) => workspaces
                .into_iter()
                .map(|workspace| workspace.id)
                .collect(),
            Err(error) => {
                ora_warn!(error = %error, "failed to list workspaces for session history cleanup");
                return Vec::new();
            }
        };
    visible_sessions(pool)
        .into_iter()
        .filter(|session| workspace_ids.contains(&session.workspace_id))
        .map(|session| session.id)
        .collect()
}

/// Lists every visible session, treating a lookup failure as nothing to clean up.
///
/// A failure here costs orphaned files, which is recoverable; propagating it
/// would block a deletion the user asked for over a bookkeeping concern.
fn visible_sessions(pool: &RepositoryPool) -> Vec<Session> {
    match SqliteSessionRepository::new(pool.clone()).list_sessions() {
        Ok(sessions) => sessions,
        Err(error) => {
            ora_warn!(error = %error, "failed to list sessions for history cleanup");
            Vec::new()
        }
    }
}
