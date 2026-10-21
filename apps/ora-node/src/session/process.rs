//! A declared model adapter cannot inherit management-host credentials or routing.

use ora_process::{ProcessEnvironment, ProcessIdentity, TokioProcessSpawner};

/// Legacy desktop-style agents retain ambient behavior; declared adapters get isolated state.
#[derive(Clone, Copy)]
pub(super) enum ModelProcessIsolation {
    Ambient,
    Isolated,
}

impl ModelProcessIsolation {
    /// The plugin and every host-managed child share this execution's environment boundary.
    pub(super) fn spawner(self, identity: ProcessIdentity) -> TokioProcessSpawner {
        let environment = match self {
            Self::Ambient => ProcessEnvironment::Inherit,
            Self::Isolated => ProcessEnvironment::Empty,
        };
        TokioProcessSpawner::running_as(identity).with_environment(environment)
    }

    /// Copies only non-authoritative process necessities into the explicitly supplied env.
    pub(super) fn extend_environment(
        self,
        variables: &mut std::collections::BTreeMap<String, String>,
    ) {
        if let Self::Isolated = self {
            for name in ["PATH", "LANG", "LC_ALL", "TMPDIR"] {
                if let Ok(value) = std::env::var(name) {
                    variables.insert(name.into(), value);
                }
            }
        }
    }
}
