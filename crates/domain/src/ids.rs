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
define_id!(TaskId, "Identifies a persisted task.");
define_id!(WorktreeId, "Identifies a persisted worktree.");
define_id!(GitCleanupJobId, "Identifies a persisted Git cleanup job.");
define_id!(
    WorktreeProvisioningLeaseId,
    "Identifies a persisted worktree provisioning lease."
);
define_id!(SessionId, "Identifies a persisted session.");
define_id!(SkillId, "Identifies a persisted skill.");
define_id!(
    AgentDefinitionId,
    "Identifies a persisted configurable agent definition."
);
define_id!(WorkflowId, "Identifies a persisted workflow.");
define_id!(
    WorkflowSnapshotId,
    "Identifies a persisted workflow snapshot."
);
define_id!(WorkflowRunId, "Identifies a persisted workflow run.");
define_id!(
    WorkflowNodeRunId,
    "Identifies a persisted workflow node run."
);
