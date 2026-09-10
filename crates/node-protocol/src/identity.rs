use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

macro_rules! string_identity {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Returns the stable wire representation.
            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub(crate) fn is_empty(&self) -> bool {
                self.0.trim().is_empty()
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

string_identity!(
    ControllerId,
    "Persistent identity of the Controller opening a Node session."
);
string_identity!(NodeId, "Persistent identity of one execution Node.");
string_identity!(
    NodeIncarnationId,
    "Identity of one running incarnation of a Node."
);
string_identity!(
    RequestId,
    "Identity of the logical Client command associated with an execution."
);
string_identity!(
    OperationId,
    "Identity of one Controller-owned business operation."
);
string_identity!(
    ExecutionId,
    "Identity of one Node execution attempt for an operation."
);
string_identity!(WorkspaceId, "Identity of an Ora Workspace on the wire.");
string_identity!(
    WorktreeId,
    "Identity of the task worktree managed by an execution."
);

/// Version of the Controller–Node wire contract.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ProtocolVersion(u16);

impl ProtocolVersion {
    pub const fn new(value: u16) -> Self {
        Self(value)
    }

    /// Returns the numeric wire representation.
    pub const fn value(self) -> u16 {
        self.0
    }
}

/// Protocol version emitted and accepted by this crate.
pub const CURRENT_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::new(/*value*/ 1);

/// Monotonic position of an event within one execution stream.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Sequence(u64);

impl Sequence {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the numeric wire representation.
    pub const fn value(self) -> u64 {
        self.0
    }
}
