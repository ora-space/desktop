use serde::{Deserialize, Serialize};

use crate::{GuardianHostDisconnect, GuardianRunOperation, RunId, RunSpec, ScopeId};

/// The host's durable launch responsibility, not proof that a guardian accepted or executed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostRunIntent {
    pub scope: ScopeId,
    pub run: RunId,
    pub spec: RunSpec,
    pub host_disconnect: GuardianHostDisconnect,
}

impl HostRunIntent {
    /// Reuses the original identity and parameters when composing a guardian request.
    pub fn start_operation(&self) -> GuardianRunOperation {
        GuardianRunOperation::Start {
            run: self.run,
            spec: self.spec.clone(),
            host_disconnect: self.host_disconnect,
        }
    }
}
