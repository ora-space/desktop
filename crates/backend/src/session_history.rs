use ora_application::{SessionRepository, TaskRepository};
use ora_db::{RepositoryPool, SqliteSessionRepository, SqliteTaskRepository};
use ora_domain::{ProjectId, Session, SessionId, TaskId};
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

/// Collects the sessions a task cascade will remove, before it removes them.
///
/// The lookup has to happen first: once the rows are soft-deleted, nothing links
/// the files back to the task that owned them.
pub(crate) fn session_ids_for_task(pool: &RepositoryPool, task_id: &TaskId) -> Vec<SessionId> {
    visible_sessions(pool)
        .into_iter()
        .filter(|session| session.task_id == *task_id)
        .map(|session| session.id)
        .collect()
}

/// Collects the sessions a project cascade will remove, across all of its tasks.
pub(crate) fn session_ids_for_project(
    pool: &RepositoryPool,
    project_id: &ProjectId,
) -> Vec<SessionId> {
    let task_ids: Vec<TaskId> = match SqliteTaskRepository::new(pool.clone()).list_tasks() {
        Ok(tasks) => tasks
            .into_iter()
            .filter(|task| task.project_id == *project_id)
            .map(|task| task.id)
            .collect(),
        Err(error) => {
            ora_warn!(error = %error, "failed to list tasks for session history cleanup");
            return Vec::new();
        }
    };
    visible_sessions(pool)
        .into_iter()
        .filter(|session| task_ids.contains(&session.task_id))
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
