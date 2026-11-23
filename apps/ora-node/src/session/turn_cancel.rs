//! Timing of a turn cancel and the Agent versions whose stopped turns may keep their session
//! (cancel-turn D2).
//!
//! A conformance run verifies the turn boundary of one exact plugin version on one platform, so the
//! list names all three: what keeps a stopped turn's work from outliving it depends on the
//! platform, and the same version is unverified on a platform nobody has measured.
use ora_node_protocol::{PluginId, PluginVersion};
use std::time::Duration;

/// Parameters of D2 in the cancel-turn decision; the durations are provisional until measured.
#[derive(Clone, Debug)]
pub struct TurnCancelConfig {
    /// G: how long a `CancelTurn` waits for the Agent to answer, from its acceptance.
    pub grace: Duration,
    /// Q: how long a stopped turn must stay silent before the next turn is sent.
    pub quiet: Duration,
    /// Agent plugin versions whose turn boundary a conformance run verified, per platform. A
    /// cancelled or failed turn of any other Agent ends the session with
    /// `turn_boundary_unverified`.
    pub turn_boundary: Vec<TurnBoundary>,
}

/// One plugin version whose turn boundary a conformance run verified on one platform.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnBoundary {
    pub plugin: PluginId,
    pub version: PluginVersion,
    pub platform: TurnBoundaryPlatform,
}

/// The platform a turn boundary was verified on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnBoundaryPlatform {
    /// The platform this Node runs on.
    Linux,
    /// A Node on Windows would stop a turn's work by other means, Job Objects for example,
    /// measured before any version is listed for it.
    Windows,
}

impl TurnBoundaryPlatform {
    /// The platform this Node runs on; the Node is built for Linux only.
    pub fn current() -> Self {
        Self::Linux
    }
}

impl TurnCancelConfig {
    /// Whether this exact plugin version has a verified turn boundary on this Node's platform.
    pub(super) fn verifies(&self, plugin: &PluginId, version: &PluginVersion) -> bool {
        self.turn_boundary.iter().any(|boundary| {
            &boundary.plugin == plugin
                && &boundary.version == version
                && boundary.platform == TurnBoundaryPlatform::current()
        })
    }
}

impl Default for TurnCancelConfig {
    /// G 15 s and Q 2 s as first set by the decision; no Agent is verified yet, so every stop of
    /// a running turn ends the session until conformance runs add versions.
    fn default() -> Self {
        Self {
            grace: Duration::from_secs(15),
            quiet: Duration::from_secs(2),
            turn_boundary: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TurnBoundary, TurnBoundaryPlatform, TurnCancelConfig};
    use ora_node_protocol::{PluginId, PluginVersion};
    use pretty_assertions::assert_eq;

    /// Only the exact plugin, version and platform of an entry are verified: another version of
    /// a listed plugin, an unlisted plugin, or a version listed only for another platform is not.
    #[test]
    fn a_boundary_is_verified_for_its_exact_version_and_platform() {
        let mut config = TurnCancelConfig::default();
        config.turn_boundary.push(TurnBoundary {
            plugin: PluginId::new("official/ora-space.opencode"),
            version: PluginVersion::new("0.6.11"),
            platform: TurnBoundaryPlatform::Linux,
        });
        config.turn_boundary.push(TurnBoundary {
            plugin: PluginId::new("official/ora-space.windows-only"),
            version: PluginVersion::new("1.0.0"),
            platform: TurnBoundaryPlatform::Windows,
        });
        assert_eq!(
            [
                ("official/ora-space.opencode", "0.6.11"),
                ("official/ora-space.opencode", "0.6.12"),
                ("official/ora-space.claude", "0.4.5"),
                ("official/ora-space.windows-only", "1.0.0"),
            ]
            .map(|(plugin, version)| {
                config.verifies(&PluginId::new(plugin), &PluginVersion::new(version))
            }),
            [true, false, false, false]
        );
    }
}
