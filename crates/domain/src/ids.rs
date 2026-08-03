use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

macro_rules! define_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

define_id!(ProjectId, "Identifies a persisted project.");
define_id!(
    ProjectWorkContextId,
    "Identifies a persisted project work context."
);
define_id!(TaskId, "Identifies a persisted task.");
define_id!(WorktreeId, "Identifies a persisted worktree.");
define_id!(
    TaskDiffCommentId,
    "Identifies a persisted task diff comment."
);
define_id!(VirtualFolderId, "Identifies a persisted virtual folder.");
define_id!(VirtualEntryId, "Identifies a persisted virtual entry.");
define_id!(SessionId, "Identifies a persisted session.");
define_id!(ArtifactId, "Identifies a persisted artifact.");
define_id!(SkillId, "Identifies a persisted skill.");
define_id!(
    AgentDefinitionId,
    "Identifies a persisted configurable agent definition."
);
