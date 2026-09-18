use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{CleanupState, DirectProcessState, OutputPolicy, RunId};

/// The policy chosen before the direct process can leave descendants behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DescendantPolicy {
    Cleanup { grace: Duration },
    WaitForAll,
}

/// Local cooperative owner liveness is a cleanup trigger, never authority to recover resources.
/// Recovery must still observe the original Run's cleanup; numeric owner identity is never signaled.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunLifetime {
    #[default]
    Independent,
    TerminateOnOwnerExit {
        pid: u32,
        start_ticks: u64,
    },
}

/// An exact local launch specification shared by the runtime and guardian wire.
///
/// OS strings preserve non-UTF-8 inputs; callers must treat persisted specifications as private data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    #[serde(
        serialize_with = "ora_utils::path::serialize_native_path",
        deserialize_with = "ora_utils::path::deserialize_native_path"
    )]
    pub cwd: PathBuf,
    pub env: BTreeMap<OsString, OsString>,
    pub descendants: DescendantPolicy,
    pub output: OutputPolicy,
    #[serde(default)]
    pub lifetime: RunLifetime,
}

impl RunSpec {
    /// Requires an explicit exit policy so this layer does not invent a deployment's grace period.
    pub fn new(
        program: impl Into<OsString>,
        cwd: impl Into<PathBuf>,
        descendants: DescendantPolicy,
    ) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            cwd: cwd.into(),
            env: BTreeMap::new(),
            descendants,
            output: OutputPolicy::Discard,
            lifetime: RunLifetime::Independent,
        }
    }
}

/// What is known about whether this attempt ever executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchFact {
    NotStarted(String),
    Started,
    Unknown(String),
}

/// Facts exposed to the guardian's caller, not an acknowledgement of durable acceptance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSnapshot {
    pub id: RunId,
    pub launch: LaunchFact,
    pub direct: DirectProcessState,
    pub cleanup: CleanupState,
}
