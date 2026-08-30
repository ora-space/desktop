//! The user-visible MCP Application State, derived from durable Effect state.
//!
//! CONTEXT.md defines the MCP Application State as the user-visible state of an MCP after
//! considering configuration completeness, compatible Agent availability, surface convergence, and
//! Agent activation: `NeedsConfiguration`, `WaitingForAgent`, `Applying`, `Ready`, or `Failed`.
//! This module is the pure read-model that folds the durable surface status, per-consumer
//! statuses, desired-set presence, and a live Agent-availability fact into that one coarse state.
//! The derivation owns no I/O; the application layer assembles its inputs from the repository and
//! the plugin runtime, so the fold is unit-testable without a database or a running agent.

use crate::{ConsumerStatus, SurfacePhase, SurfaceStatus};

/// The five user-visible states an MCP traverses, per CONTEXT.md.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum McpApplicationState {
    /// The workspace has no MCP desired set, so there is nothing to apply or activate.
    NeedsConfiguration,
    /// An MCP is desired but no compatible Agent process is running to render and consume it.
    WaitingForAgent,
    /// A compatible Agent is running and the surface is converging: rendering, writing, or resuming.
    Applying,
    /// The file is applied and the consuming Agent is current at the desired generation.
    Ready,
    /// A durable condition the user must resolve: a foreign file Ora does not own, a failed
    /// materialization, or an Agent that did not resume after the file it should have consumed.
    Failed,
}

/// The facts the derivation folds into one MCP Application State.
///
/// Each field is a fact the application layer can already read independently — desired-set
/// presence and statuses from the repository, and a running-consumer fact from the plugin runtime
/// — so the fold stays free of I/O and therefore unit-testable in isolation.
pub struct McpApplicationStateInput<'a> {
    /// Whether the workspace effect spec declares any MCP desired state at all.
    pub has_desired: bool,
    /// The durable status of the MCP surface, or `None` when no reconcile has recorded one yet.
    pub surface: Option<&'a SurfaceStatus>,
    /// Every per-consumer readiness row recorded for the MCP surface.
    pub consumers: &'a [ConsumerStatus],
    /// Whether a compatible Agent process is currently running to serve the surface's renderer.
    pub agent_running: bool,
}

/// Folds durable Effect state and a live Agent-availability fact into one MCP Application State.
///
/// The precedence mirrors how a user reads the state: nothing configured before no agent, no agent
/// before not-yet-applied, applied-and-current before any failure, and a durable failure before the
/// transient convergence that would otherwise mask it.
pub fn derive_mcp_application_state(input: McpApplicationStateInput<'_>) -> McpApplicationState {
    if !input.has_desired {
        return McpApplicationState::NeedsConfiguration;
    }
    if !input.agent_running {
        return McpApplicationState::WaitingForAgent;
    }
    let Some(surface) = input.surface else {
        // An Agent is running and an MCP is desired, but no surface status has been recorded yet:
        // the worker is about to render and write, so the surface is in-flight convergence.
        return McpApplicationState::Applying;
    };
    // A durable failure the user must resolve outranks the transient convergence that produced it.
    if surface.phase == SurfacePhase::Degraded || surface.phase == SurfacePhase::RecoveryRequired {
        return McpApplicationState::Failed;
    }
    if input
        .consumers
        .iter()
        .any(|consumer| consumer.phase == SurfacePhase::Degraded)
    {
        return McpApplicationState::Failed;
    }
    // The file is applied at the desired generation and every consumer resumed current at it; an
    // empty consumer set is vacuously current, which is how the idempotent no-op path (already
    // applied, file still Ora-owned) reads as Ready without a freshly-recorded consumer row.
    if surface.applied_generation >= surface.desired_generation
        && input
            .consumers
            .iter()
            .all(|consumer| consumer.phase == SurfacePhase::Current)
    {
        return McpApplicationState::Ready;
    }
    McpApplicationState::Applying
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Condition, ConditionReason, ConditionSubject, Generation, SurfaceKey};
    use ora_domain::WorkspaceId;
    use pretty_assertions::assert_eq;

    fn workspace_id() -> WorkspaceId {
        WorkspaceId::new("ws-app-state")
    }

    fn surface_key() -> SurfaceKey {
        SurfaceKey::new("mcp")
    }

    /// Builds a surface status with the fields the derivation reads, filling the rest with the
    /// values the reconcile persist path itself uses, so tests stay aligned with production rows.
    fn surface_status(
        phase: SurfacePhase,
        applied: Generation,
        conditions: Vec<Condition>,
    ) -> SurfaceStatus {
        SurfaceStatus {
            workspace_id: workspace_id(),
            surface_key: surface_key(),
            desired_generation: Generation::new(1),
            observed_generation: Generation::new(1),
            applied_generation: applied,
            phase,
            revision: 1,
            updated_at: 0,
            conditions,
        }
    }

    fn consumer_status(phase: SurfacePhase) -> ConsumerStatus {
        ConsumerStatus {
            surface_key: surface_key(),
            consumer_id: crate::ConsumerId::new("official/ora-space.opencode"),
            ready_generation: Generation::new(1),
            phase,
            revision: 1,
            updated_at: 0,
            conditions: Vec::new(),
        }
    }

    fn ownership_conflict_condition() -> Condition {
        Condition::new(
            ConditionSubject::Surface {
                surface_key: surface_key(),
            },
            ConditionReason::OwnershipConflict,
            "a foreign file Ora does not own blocks the surface",
            0,
            Generation::new(1),
        )
    }

    fn input<'a>(
        has_desired: bool,
        surface: Option<&'a SurfaceStatus>,
        consumers: &'a [ConsumerStatus],
        agent_running: bool,
    ) -> McpApplicationStateInput<'a> {
        McpApplicationStateInput {
            has_desired,
            surface,
            consumers,
            agent_running,
        }
    }

    /// A workspace with no MCP desired set has nothing to apply or activate.
    #[test]
    fn no_desired_means_needs_configuration() {
        let state = derive_mcp_application_state(input(false, None, &[], false));
        assert_eq!(state, McpApplicationState::NeedsConfiguration);
    }

    /// A desired MCP with no compatible Agent running waits for one before it can converge.
    #[test]
    fn desired_without_running_agent_waits_for_agent() {
        let surface = surface_status(SurfacePhase::Pending, Generation::default(), Vec::new());
        let state = derive_mcp_application_state(input(true, Some(&surface), &[], false));
        assert_eq!(state, McpApplicationState::WaitingForAgent);
    }

    /// A file applied at the desired generation whose consumer resumed current is ready to use.
    #[test]
    fn applied_file_with_current_consumer_is_ready() {
        let surface = surface_status(SurfacePhase::Current, Generation::new(1), Vec::new());
        let consumer = consumer_status(SurfacePhase::Current);
        let state = derive_mcp_application_state(input(true, Some(&surface), &[consumer], true));
        assert_eq!(state, McpApplicationState::Ready);
    }

    /// A surface whose own materialization degraded is failed regardless of consumer readiness.
    #[test]
    fn degraded_surface_is_failed() {
        let surface = surface_status(SurfacePhase::Degraded, Generation::new(1), Vec::new());
        let consumer = consumer_status(SurfacePhase::Current);
        let state = derive_mcp_application_state(input(true, Some(&surface), &[consumer], true));
        assert_eq!(state, McpApplicationState::Failed);
    }

    /// A foreign file Ora does not own parks the surface for a human and reads as failed.
    #[test]
    fn foreign_file_ownership_conflict_is_failed() {
        let surface = surface_status(
            SurfacePhase::RecoveryRequired,
            Generation::default(),
            vec![ownership_conflict_condition()],
        );
        let state = derive_mcp_application_state(input(true, Some(&surface), &[], true));
        assert_eq!(state, McpApplicationState::Failed);
    }

    /// A file written for the desired generation whose Agent did not resume is failed, not ready.
    #[test]
    fn applied_file_with_degraded_consumer_is_failed() {
        let surface = surface_status(SurfacePhase::Current, Generation::new(1), Vec::new());
        let consumer = consumer_status(SurfacePhase::Degraded);
        let state = derive_mcp_application_state(input(true, Some(&surface), &[consumer], true));
        assert_eq!(state, McpApplicationState::Failed);
    }

    /// A running Agent with a desired MCP whose file is not yet applied is still converging.
    #[test]
    fn in_flight_surface_is_applying() {
        let surface = surface_status(
            SurfacePhase::WaitingForIdle,
            Generation::default(),
            Vec::new(),
        );
        let state = derive_mcp_application_state(input(true, Some(&surface), &[], true));
        assert_eq!(state, McpApplicationState::Applying);
    }

    /// A running Agent with a desired MCP but no recorded surface status is about to converge.
    #[test]
    fn desired_with_agent_but_no_surface_status_is_applying() {
        let state = derive_mcp_application_state(input(true, None, &[], true));
        assert_eq!(state, McpApplicationState::Applying);
    }
}
