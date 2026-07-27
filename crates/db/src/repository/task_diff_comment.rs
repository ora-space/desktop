use ora_application::{TaskDiffCommentRepository, TaskDiffCommentRepositoryError};
use ora_domain::{
    AuditFields, TaskDiffAnchor, TaskDiffComment, TaskDiffCommentId, TaskDiffCommentKind,
    TaskDiffSide, TaskDiffThreadStatus, TaskId,
};
use rusqlite::{Row, params};

use crate::repository::{RepositoryPool, connection::bool_to_sqlite};

/// Persists task diff root discussions and replies through SQLite.
#[derive(Clone, Debug)]
pub struct SqliteTaskDiffCommentRepository {
    pool: RepositoryPool,
}

impl SqliteTaskDiffCommentRepository {
    /// Builds a task diff comment repository from the shared pool.
    pub fn new(pool: RepositoryPool) -> Self {
        Self { pool }
    }
}

impl TaskDiffCommentRepository for SqliteTaskDiffCommentRepository {
    /// Inserts one root discussion or reply and returns the stored snapshot.
    fn create_comment(
        &self,
        comment: TaskDiffComment,
    ) -> Result<TaskDiffComment, TaskDiffCommentRepositoryError> {
        let columns = comment_columns(&comment);
        self.pool
            .with_connection(|connection| {
                connection.execute(
                    "INSERT INTO task_diff_comments (
                        id, task_id, parent_comment_id, diff_id, path, side, start_line, end_line,
                        hunk_header, line_content, body, status, created_at, updated_at, is_deleted
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                    params![
                        comment.id.as_ref(),
                        comment.task_id.as_ref(),
                        columns.parent_comment_id,
                        columns.diff_id,
                        columns.path,
                        columns.side,
                        columns.start_line,
                        columns.end_line,
                        columns.hunk_header,
                        columns.line_content,
                        &comment.body,
                        columns.status,
                        comment.audit_fields.created_at,
                        comment.audit_fields.updated_at,
                        bool_to_sqlite(comment.audit_fields.is_deleted),
                    ],
                )?;

                Ok(())
            })
            .map(|()| comment)
            .map_err(comment_repository_error_from_database)
    }

    /// Loads one visible discussion message by identifier.
    fn find_comment(
        &self,
        comment_id: &TaskDiffCommentId,
    ) -> Result<Option<TaskDiffComment>, TaskDiffCommentRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(COMMENT_SELECT_BY_ID)?;
                let mut rows = statement.query(params![comment_id.as_ref()])?;

                match rows.next()? {
                    Some(row) => Ok(Some(map_comment_row(row)?)),
                    None => Ok(None),
                }
            })
            .map_err(comment_repository_error_from_database)
    }

    /// Lists visible root discussions and replies in stable creation order.
    fn list_comments(
        &self,
        task_id: &TaskId,
    ) -> Result<Vec<TaskDiffComment>, TaskDiffCommentRepositoryError> {
        self.pool
            .with_connection(|connection| {
                let mut statement = connection.prepare(COMMENT_SELECT_BY_TASK)?;
                let mut rows = statement.query(params![task_id.as_ref()])?;
                let mut comments = Vec::new();

                while let Some(row) = rows.next()? {
                    comments.push(map_comment_row(row)?);
                }

                Ok(comments)
            })
            .map_err(comment_repository_error_from_database)
    }

    /// Persists a root discussion status replacement.
    fn update_comment(
        &self,
        comment: TaskDiffComment,
    ) -> Result<TaskDiffComment, TaskDiffCommentRepositoryError> {
        let status = match &comment.kind {
            TaskDiffCommentKind::Thread { status, .. } => status.database_value(),
            TaskDiffCommentKind::Reply { .. } => {
                return Err(TaskDiffCommentRepositoryError::OperationFailed(
                    "reply comments do not own resolution state".to_string(),
                ));
            }
        };

        self.pool
            .with_connection(|connection| {
                let updated_rows = connection.execute(
                    "UPDATE task_diff_comments
                     SET status = ?2, updated_at = ?3
                     WHERE id = ?1 AND parent_comment_id IS NULL AND is_deleted = 0",
                    params![comment.id.as_ref(), status, comment.audit_fields.updated_at],
                )?;
                if updated_rows == 0 {
                    return Err(crate::DatabaseError::Sqlite(
                        rusqlite::Error::QueryReturnedNoRows,
                    ));
                }

                Ok(comment)
            })
            .map_err(comment_repository_error_from_database)
    }
}

const COMMENT_SELECT_BY_ID: &str = "SELECT id, task_id, parent_comment_id, diff_id, path, side, start_line, end_line, hunk_header, line_content, body, status, created_at, updated_at, is_deleted FROM task_diff_comments WHERE id = ?1 AND is_deleted = 0";
const COMMENT_SELECT_BY_TASK: &str = "SELECT id, task_id, parent_comment_id, diff_id, path, side, start_line, end_line, hunk_header, line_content, body, status, created_at, updated_at, is_deleted FROM task_diff_comments WHERE task_id = ?1 AND is_deleted = 0 ORDER BY created_at, id";

/// Holds the nullable database columns corresponding to the domain comment kind.
struct CommentColumns<'a> {
    parent_comment_id: Option<&'a str>,
    diff_id: Option<&'a str>,
    path: Option<&'a str>,
    side: Option<i64>,
    start_line: Option<i64>,
    end_line: Option<i64>,
    hunk_header: Option<&'a str>,
    line_content: Option<&'a str>,
    status: Option<i64>,
}

/// Converts a domain kind into the mutually exclusive nullable SQLite column groups.
fn comment_columns(comment: &TaskDiffComment) -> CommentColumns<'_> {
    match &comment.kind {
        TaskDiffCommentKind::Thread { anchor, status } => CommentColumns {
            parent_comment_id: None,
            diff_id: Some(&anchor.diff_id),
            path: Some(&anchor.path),
            side: Some(anchor.side.database_value()),
            start_line: Some(i64::from(anchor.start_line)),
            end_line: Some(i64::from(anchor.end_line)),
            hunk_header: Some(&anchor.hunk_header),
            line_content: Some(&anchor.line_content),
            status: Some(status.database_value()),
        },
        TaskDiffCommentKind::Reply { parent_comment_id } => CommentColumns {
            parent_comment_id: Some(parent_comment_id.as_ref()),
            diff_id: None,
            path: None,
            side: None,
            start_line: None,
            end_line: None,
            hunk_header: None,
            line_content: None,
            status: None,
        },
    }
}

/// Reconstructs one valid domain comment from its mutually exclusive column groups.
fn map_comment_row(row: &Row<'_>) -> Result<TaskDiffComment, crate::DatabaseError> {
    let parent_comment_id = row.get::<_, Option<String>>("parent_comment_id")?;
    let kind = match parent_comment_id {
        Some(parent_comment_id) => TaskDiffCommentKind::Reply {
            parent_comment_id: TaskDiffCommentId::new(parent_comment_id),
        },
        None => {
            let side = row
                .get::<_, Option<i64>>("side")?
                .and_then(TaskDiffSide::from_database_value)
                .ok_or_else(invalid_comment_row)?;
            let status = row
                .get::<_, Option<i64>>("status")?
                .and_then(TaskDiffThreadStatus::from_database_value)
                .ok_or_else(invalid_comment_row)?;
            let start_line = u32::try_from(
                row.get::<_, Option<i64>>("start_line")?
                    .ok_or_else(invalid_comment_row)?,
            )
            .map_err(|_| invalid_comment_row())?;
            let end_line = u32::try_from(
                row.get::<_, Option<i64>>("end_line")?
                    .ok_or_else(invalid_comment_row)?,
            )
            .map_err(|_| invalid_comment_row())?;

            TaskDiffCommentKind::Thread {
                anchor: TaskDiffAnchor {
                    diff_id: row
                        .get::<_, Option<String>>("diff_id")?
                        .ok_or_else(invalid_comment_row)?,
                    path: row
                        .get::<_, Option<String>>("path")?
                        .ok_or_else(invalid_comment_row)?,
                    side,
                    start_line,
                    end_line,
                    hunk_header: row
                        .get::<_, Option<String>>("hunk_header")?
                        .ok_or_else(invalid_comment_row)?,
                    line_content: row
                        .get::<_, Option<String>>("line_content")?
                        .ok_or_else(invalid_comment_row)?,
                },
                status,
            }
        }
    };
    let is_deleted = row.get::<_, i64>("is_deleted")? != 0;

    Ok(TaskDiffComment::new(
        TaskDiffCommentId::new(row.get::<_, String>("id")?),
        TaskId::new(row.get::<_, String>("task_id")?),
        kind,
        row.get::<_, String>("body")?,
        AuditFields::new(row.get("created_at")?, row.get("updated_at")?, is_deleted),
    ))
}

/// Constructs a deterministic database error for malformed mutually exclusive comment columns.
fn invalid_comment_row() -> crate::DatabaseError {
    crate::DatabaseError::Sqlite(rusqlite::Error::InvalidQuery)
}

/// Converts database-layer failures into the application repository error.
fn comment_repository_error_from_database(
    error: crate::DatabaseError,
) -> TaskDiffCommentRepositoryError {
    TaskDiffCommentRepositoryError::OperationFailed(error.to_string())
}
