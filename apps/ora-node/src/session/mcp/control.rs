//! The memory-only update and admission boundary of one execution. No Settings enter a journal.
use ora_node_protocol::*;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use tokio::sync::Notify;
#[cfg(test)]
mod tests;

/// One retained complete input; cloning shares values instead of creating another secret snapshot.
pub(crate) struct Input {
    pub generation: u64,
    pub members: Vec<SessionMcpUpdateMember>,
}

impl Input {
    /// Projects observations without values or hashes of values.
    pub fn identity(&self) -> SessionMcpRevisionIdentity {
        SessionMcpRevisionIdentity {
            generation: self.generation,
            members: self
                .members
                .iter()
                .map(SessionMcpUpdateMember::identity)
                .collect(),
        }
    }
}

/// A clone addresses the same atomic desired/admission order.
#[derive(Clone)]
pub(crate) struct Control {
    state: Arc<Mutex<State>>,
    pub changed: Arc<Notify>,
    pub refresh: Arc<Notify>,
    pub probes: Arc<Notify>,
}

struct State {
    desired: Arc<Input>,
    active: Option<Arc<Input>>,
    sending: Option<Arc<Input>>,
    version: u64,
    convergence: SessionMcpConvergence,
    failure: Option<SessionMcpFailure>,
    health: Vec<SessionMcpMemberState>,
    flights: std::collections::BTreeMap<PluginId, Arc<super::probes::ProbeInput>>,
    automatic: std::collections::BTreeSet<PluginId>,
}

impl Control {
    /// Counts value-free queue and cleanup facts for the Node's root-only diagnostics.
    pub(super) fn probe_counts(&self) -> (usize, usize) {
        let state = self.lock();
        (
            state.health.iter().filter(|member| member.queued).count(),
            state
                .health
                .iter()
                .filter(|member| member.cleanup_pending)
                .count(),
        )
    }
    /// Successful host resolution queues at most one automatic observation per member/generation.
    pub(super) fn resolved(&self, generation: u64) {
        let mut state = self.lock();
        if state.desired.generation != generation
            || matches!(
                state.convergence,
                SessionMcpConvergence::Stopping | SessionMcpConvergence::Ended
            )
        {
            return;
        }
        let ids: Vec<_> = state
            .desired
            .members
            .iter()
            .filter(|member| matches!(member, SessionMcpUpdateMember::Configured { .. }))
            .map(|member| member.plugin_id().clone())
            .collect();
        for id in ids {
            if state.automatic.insert(id.clone())
                && let Some(member) = state
                    .health
                    .iter_mut()
                    .find(|member| member.plugin_id == id)
                && matches!(
                    member.health,
                    SessionMcpHealth::Unknown {
                        reason: SessionMcpUnknownReason::NotProbed
                    }
                )
            {
                member.queued = true;
            }
        }
        bump(&mut state);
        drop(state);
        self.probes.notify_one();
    }
    /// Claims one coalesced intent only when the global scheduler has capacity.
    pub(super) fn claim_probe(&self) -> Option<Arc<super::probes::ProbeInput>> {
        let mut state = self.lock();
        if matches!(
            state.convergence,
            SessionMcpConvergence::Stopping | SessionMcpConvergence::Ended
        ) {
            return None;
        }
        let index = state
            .health
            .iter()
            .position(|member| member.queued && !state.flights.contains_key(&member.plugin_id))?;
        let input = Arc::new(super::probes::ProbeInput {
            generation: state.desired.generation,
            attempt: state.health[index].probe_attempt,
            member: state.desired.members[index].clone(),
        });
        state
            .flights
            .insert(input.member.plugin_id().clone(), Arc::clone(&input));
        state.health[index].queued = false;
        state.health[index].probe_in_flight = true;
        bump(&mut state);
        Some(input)
    }

    /// Late completions cannot update another generation, configuration or manual attempt.
    pub(super) fn probe_result(
        &self,
        input: &super::probes::ProbeInput,
        health: SessionMcpHealth,
        cleanup_pending: bool,
    ) {
        let mut state = self.lock();
        if !cleanup_pending {
            state.flights.remove(input.member.plugin_id());
        }
        if state.desired.generation != input.generation {
            if let Some(member) = state
                .health
                .iter_mut()
                .find(|member| &member.plugin_id == input.member.plugin_id())
            {
                member.probe_in_flight = cleanup_pending;
                member.cleanup_pending = cleanup_pending;
            }
            bump(&mut state);
            return;
        }
        let Some(index) = state
            .desired
            .members
            .iter()
            .position(|member| member == &input.member)
        else {
            return;
        };
        let member = &mut state.health[index];
        member.cleanup_pending = cleanup_pending;
        member.probe_in_flight = cleanup_pending;
        // Larger manual counters join this same configuration's in-flight observation.
        if member.probe_attempt >= input.attempt {
            member.health = health;
        }
        bump(&mut state);
    }
    /// A detached provider can be restored only by a future load of its original identity.
    pub(super) fn detached(&self) {
        let mut state = self.lock();
        if state.convergence != SessionMcpConvergence::Stopping {
            state.convergence = SessionMcpConvergence::Pending;
        }
        bump(&mut state);
        drop(state);
        self.changed.notify_one();
    }

    /// Resolves the attempted generation without confusing a late failure with the newest target.
    pub(crate) fn failed(&self, failure: SessionMcpFailure) {
        let generation = {
            let state = self.lock();
            state.sending.as_ref().unwrap_or(&state.desired).generation
        };
        self.blocked(generation, failure);
    }
    /// Coalesces one value-free intent per current member; repeated attempts never create flights.
    pub(crate) fn probe(&self, request: &ProbeSessionMcp) -> SessionMcpResult {
        let mut state = self.lock();
        if matches!(
            state.convergence,
            SessionMcpConvergence::Stopping | SessionMcpConvergence::Ended
        ) {
            return SessionMcpResult::SessionEnded;
        }
        if request.generation != state.desired.generation {
            return SessionMcpResult::Stale;
        }
        let Some(index) = state
            .desired
            .members
            .iter()
            .position(|member| member.plugin_id() == &request.plugin_id)
        else {
            return SessionMcpResult::InvalidSelection;
        };
        if matches!(
            state.desired.members[index],
            SessionMcpUpdateMember::Unavailable { .. }
        ) {
            return SessionMcpResult::NotConfigured;
        }
        let coalesced = state
            .flights
            .get(&request.plugin_id)
            .is_some_and(|flight| flight.generation == request.generation);
        let member = &mut state.health[index];
        if request.probe_attempt == member.probe_attempt {
            return SessionMcpResult::Duplicate;
        }
        if request.probe_attempt < member.probe_attempt {
            return SessionMcpResult::Stale;
        }
        member.probe_attempt = request.probe_attempt;
        member.queued = !coalesced;
        bump(&mut state);
        drop(state);
        self.changed.notify_one();
        self.probes.notify_one();
        SessionMcpResult::Accepted
    }
    /// Creates generation zero from the immutable authorization selected at session start.
    pub(crate) fn new(bindings: &[SessionMcpBinding]) -> Self {
        let members: Vec<_> = bindings
            .iter()
            .map(|binding| SessionMcpUpdateMember::Configured {
                plugin_id: binding.plugin_id.clone(),
                plugin_version: binding.plugin_version.clone(),
                configuration_revision: 0,
                settings: binding.settings.clone(),
            })
            .collect();
        let health = unknown(&members);
        Self {
            state: Arc::new(Mutex::new(State {
                desired: Arc::new(Input {
                    generation: 0,
                    members,
                }),
                active: None,
                sending: None,
                version: 0,
                convergence: SessionMcpConvergence::Pending,
                failure: None,
                health,
                flights: Default::default(),
                automatic: Default::default(),
            })),
            changed: Arc::new(Notify::new()),
            refresh: Arc::new(Notify::new()),
            probes: Arc::new(Notify::new()),
        }
    }

    /// Serializes input comparison, target publication, and final prompt admission.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Does not resolve packages or initiate external work; accepted means the new target is visible.
    pub(crate) fn update(
        &self,
        update: &UpdateSessionMcp,
    ) -> (SessionMcpResult, Option<SessionMcpErrorCode>) {
        let mut state = self.lock();
        if matches!(
            state.convergence,
            SessionMcpConvergence::Stopping | SessionMcpConvergence::Ended
        ) {
            return (SessionMcpResult::SessionEnded, None);
        }
        if update.generation < state.desired.generation {
            return (SessionMcpResult::Stale, None);
        }
        if update.generation == state.desired.generation {
            return (
                if update.members == state.desired.members {
                    SessionMcpResult::Duplicate
                } else {
                    SessionMcpResult::Conflict
                },
                None,
            );
        }
        if !update
            .members
            .iter()
            .map(SessionMcpUpdateMember::plugin_id)
            .eq(state
                .desired
                .members
                .iter()
                .map(SessionMcpUpdateMember::plugin_id))
        {
            return (SessionMcpResult::InvalidSelection, None);
        }
        for held in [
            Some(&state.desired),
            state.active.as_ref(),
            state.sending.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            for old in &held.members {
                if update
                    .members
                    .iter()
                    .any(|new| new.identity() == old.identity() && new != old)
                {
                    return (
                        SessionMcpResult::Conflict,
                        Some(SessionMcpErrorCode::ConfigurationRevisionConflict),
                    );
                }
            }
        }
        if state.flights.values().any(|flight| {
            update
                .members
                .iter()
                .any(|new| new.identity() == flight.member.identity() && new != &flight.member)
        }) {
            return (
                SessionMcpResult::Conflict,
                Some(SessionMcpErrorCode::ConfigurationRevisionConflict),
            );
        }
        state.desired = Arc::new(Input {
            generation: update.generation,
            members: update.members.clone(),
        });
        state.automatic.clear();
        let cleanup: Vec<_> = state
            .health
            .iter()
            .filter(|member| member.cleanup_pending)
            .map(|member| member.plugin_id.clone())
            .collect();
        state.health = unknown(&update.members);
        let in_flight: Vec<_> = state.flights.keys().cloned().collect();
        for member in &mut state.health {
            member.probe_in_flight = in_flight.contains(&member.plugin_id);
            member.cleanup_pending = cleanup.contains(&member.plugin_id);
        }
        state.convergence = SessionMcpConvergence::Pending;
        state.failure = None;

        bump(&mut state);
        drop(state);
        self.changed.notify_one();
        self.refresh.notify_one();
        (SessionMcpResult::Accepted, None)
    }

    /// A read never refreshes, parses or probes.
    pub(crate) fn snapshot(&self, capacity: bool) -> SessionMcpState {
        let state = self.lock();
        SessionMcpState {
            state_version: state.version,
            desired: state.desired.identity(),
            active: state.active.as_ref().map(|input| input.identity()),
            convergence: state.convergence,
            failure: state.failure.clone(),
            members: state.health.clone(),
            probe_capacity_available: capacity,
        }
    }

    /// Only the resolver's current in-flight input is retained beside active and latest desired.
    pub(super) fn begin(&self) -> Arc<Input> {
        let mut state = self.lock();
        let input = Arc::clone(&state.desired);
        state.sending = Some(Arc::clone(&input));
        state.convergence = SessionMcpConvergence::Refreshing;
        state.failure = None;
        bump(&mut state);
        input
    }

    /// Target inspection shares its memory and cannot expose settings in diagnostics.
    pub(super) fn desired(&self) -> Arc<Input> {
        Arc::clone(&self.lock().desired)
    }

    /// A stale success remains historical active but cannot admit a turn against a newer target.
    pub(super) fn loaded(&self, generation: u64) {
        let mut state = self.lock();
        let Some(input) = state
            .sending
            .as_ref()
            .filter(|input| input.generation == generation)
            .cloned()
        else {
            return;
        };
        state.active = Some(input);
        state.sending = None;
        state.convergence = if state.desired.generation == generation {
            SessionMcpConvergence::Current
        } else {
            SessionMcpConvergence::Pending
        };
        state.failure = None;
        bump(&mut state);
        drop(state);
        self.changed.notify_one();
        self.resolved(generation);
    }

    /// Failure never destroys the last delivered identity or rolls the target back.
    pub(super) fn blocked(&self, generation: u64, failure: SessionMcpFailure) {
        let mut state = self.lock();
        if state
            .sending
            .as_ref()
            .is_some_and(|input| input.generation == generation)
        {
            state.sending = None;
        }
        if state.desired.generation != generation {
            return;
        }
        state.convergence = SessionMcpConvergence::Blocked;
        state.failure = Some(failure);
        bump(&mut state);
        drop(state);
        self.changed.notify_one();
    }

    /// The host commits while holding the same lock as Update's target replacement.
    pub(crate) fn admit<T>(&self, commit: impl FnOnce() -> T) -> Result<T, Admission> {
        let state = self.lock();
        if state.convergence != SessionMcpConvergence::Current
            || state
                .active
                .as_ref()
                .is_none_or(|active| active.generation != state.desired.generation)
        {
            return Err(if state.convergence == SessionMcpConvergence::Blocked {
                Admission::Blocked
            } else {
                Admission::Pending
            });
        }
        Ok(commit())
    }

    /// Stop admission immediately; resource owners continue independently until proven stopped.
    pub(crate) fn stop(&self) {
        let mut state = self.lock();
        state.convergence = SessionMcpConvergence::Stopping;
        bump(&mut state);
        drop(state);
        self.changed.notify_one();
    }
}

/// Non-terminal readiness results are typed separately from provider and storage failures.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Admission {
    #[error("mcp_admission_pending")]
    Pending,
    #[error("mcp_admission_blocked")]
    Blocked,
}

/// Generation changes invalidate all previous host health without reusing remote results.
fn unknown(members: &[SessionMcpUpdateMember]) -> Vec<SessionMcpMemberState> {
    members
        .iter()
        .map(|member| SessionMcpMemberState {
            plugin_id: member.plugin_id().clone(),
            health: SessionMcpHealth::Unknown {
                reason: SessionMcpUnknownReason::NotProbed,
            },
            probe_attempt: 0,
            probe_in_flight: false,
            queued: false,
            cleanup_pending: false,
        })
        .collect()
}

/// State versions stay exactly representable to JSON consumers.
fn bump(state: &mut State) {
    state.version = state.version.saturating_add(1).min(MAX_SESSION_MCP_COUNTER);
}
