use serde::{Deserialize, Serialize};

/// Carries persistence-managed audit fields shared by every schema-backed entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditFields {
    pub created_at: i64,
    pub updated_at: i64,
    pub is_deleted: bool,
}

impl AuditFields {
    /// Creates audit metadata using Unix timestamps in milliseconds plus a soft-delete flag.
    pub fn new(created_at: i64, updated_at: i64, is_deleted: bool) -> Self {
        Self {
            created_at,
            updated_at,
            is_deleted,
        }
    }
}
