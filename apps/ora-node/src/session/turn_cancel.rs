//! Timing of a turn cancel and the Agent versions whose stopped turns may keep their session
//! (cancel-turn D2).
//!
//! A verified turn boundary is what the Agent and the Node give together: the Agent answers the
//! prompt and reports nothing more for the turn, and the Node's turn process fence kills what the
//! turn started in sessions of its own before the session is kept. A conformance run checks that
//! combination for one exact plugin version on one platform, so the list names all three: the
//! same version is unverified on a platform whose fence nobody has measured.
use ora_node_protocol::{PluginId, PluginVersion};
use std::time::Duration;

/// Parameters of D2 in the cancel-turn decision; the durations are provisional until measured.
#[derive(Clone, Debug)]
pub struct TurnCancelConfig {
    /// G: how long a `CancelTurn` waits for the Agent to answer, from its acceptance.
    pub grace: Duration,
    /// Q: how long a stopped turn must stay silent before the next turn is sent.
    pub quiet: Duration,
    /// F: how long the turn process fence may take, after the answer, to show that the turn's
    /// processes are gone. Past it the session ends with `turn_processes_unstopped`.
    pub process_fence: Duration,
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

/// The platform a turn boundary was verified on; each has its own turn process fence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnBoundaryPlatform {
    /// The attempt mark and procfs fence this Node runs.
    Linux,
    /// A Node on Windows would need a fence of its own, Job Objects for example, measured before
    /// any version is listed for it.
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
    /// G 15 s, Q 2 s and F 5 s as first set by the decisions, and the versions that passed
    /// conformance.
    fn default() -> Self {
        Self {
            grace: Duration::from_secs(15),
            quiet: Duration::from_secs(2),
            process_fence: Duration::from_secs(5),
            turn_boundary: vec![
                // OpenCode CLI 1.19.3, the package v2 deploys. Conformance: the standalone
                // `turn_boundary_conformance` test, five runs of each probe scenario (shell,
                // subagent, background, detached, error) through a Node with the turn process
                // fence on x86_64 Linux, package sha256
                // 88c00f81997fc7fe892f2d58a686f535678e562c7e000d33eb7ee3b1d0f754b5; measurements
                // in ora-space/circus#1340.
                TurnBoundary {
                    plugin: PluginId::new("official/ora-space.opencode"),
                    version: PluginVersion::new("0.6.11"),
                    platform: TurnBoundaryPlatform::Linux,
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{TurnBoundary, TurnBoundaryPlatform, TurnCancelConfig};
    use ora_node_protocol::{PluginId, PluginVersion};
    use pretty_assertions::assert_eq;

    /// Only the exact plugin, version and platform of an entry are verified: the default lists
    /// OpenCode 0.6.11 on Linux, not Claude Code 0.4.5, another OpenCode version, or a version
    /// listed only for another platform.
    #[test]
    fn a_boundary_is_verified_for_its_exact_version_and_platform() {
        let mut config = TurnCancelConfig::default();
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
