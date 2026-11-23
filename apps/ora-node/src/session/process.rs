//! A declared model adapter cannot inherit management-host credentials or routing.
//!
//! The plugin and every process the host spawns for it share one spawner, so clearing inheritance
//! there keeps provider keys, base URLs and CLI configuration variables of the Node's own
//! environment away from the adapter and its CLI alike (ADR D4).

use ora_process::{ProcessEnvironment, ProcessIdentity, TokioProcessSpawner};
use std::collections::BTreeMap;

/// Variables a process needs to run at all and that grant no authority, copied into an
/// isolated session's explicit environment.
const PROCESS_NECESSITIES: [&str; 4] = ["PATH", "LANG", "LC_ALL", "TMPDIR"];

/// Whether a session's processes inherit the Node's environment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ModelProcessIsolation {
    /// Sessions without a frozen configuration keep the existing inherited environment.
    Ambient,
    /// A configured session sees only what the session supplies explicitly.
    Isolated,
}

impl ModelProcessIsolation {
    /// Isolates exactly the sessions that run a declared adapter.
    pub(super) fn for_session(configured: bool) -> Self {
        if configured {
            Self::Isolated
        } else {
            Self::Ambient
        }
    }

    /// The spawner for the plugin and every host-managed child of this session.
    ///
    /// The Node's reaper ends the Git it runs when the Node dies. Session processes stay out of
    /// it: what happens to them after an abnormal Node exit is unchanged.
    pub(super) fn spawner(self, identity: ProcessIdentity) -> TokioProcessSpawner {
        let environment = match self {
            Self::Ambient => ProcessEnvironment::Inherit,
            Self::Isolated => ProcessEnvironment::Empty,
        };
        TokioProcessSpawner::running_as(identity)
            .with_environment(environment)
            .without_reaper_registration()
    }

    /// Copies only non-authoritative process necessities into the explicit environment; values
    /// the session already set win.
    pub(super) fn extend_environment(self, variables: &mut BTreeMap<String, String>) {
        if self == Self::Isolated {
            for name in PROCESS_NECESSITIES {
                if let Ok(value) = std::env::var(name) {
                    variables.entry(name.to_owned()).or_insert(value);
                }
            }
        }
    }
}
