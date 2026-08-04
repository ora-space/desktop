use super::ports::TaskDiffCommentIdGenerator;
use ora_domain::TaskDiffCommentId;
use uuid::Uuid;

/// Generates UUID-backed task diff comment identifiers.
#[derive(Clone, Copy, Debug, Default)]
pub struct UuidTaskDiffCommentIdGenerator;

impl UuidTaskDiffCommentIdGenerator {
    /// Builds the UUID-backed identifier generator.
    pub fn new() -> Self {
        Self
    }
}

impl TaskDiffCommentIdGenerator for UuidTaskDiffCommentIdGenerator {
    /// Produces a fresh UUID v4 comment identifier.
    fn generate_comment_id(&self) -> TaskDiffCommentId {
        TaskDiffCommentId::new(Uuid::new_v4().to_string())
    }
}
