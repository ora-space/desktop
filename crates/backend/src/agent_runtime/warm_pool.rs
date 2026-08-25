//! Bookkeeping for provider sessions created before the user's first prompt.
//!
//! ACP only reports a session's configuration options — the model selector among
//! them — as part of `session/new`. Ora therefore creates a session as soon as a
//! chat surface opens, so a model can be chosen before anything is sent. This
//! module owns which of those sessions exist, which may be reused, and which
//! must be torn down. It performs no I/O: every method returns a decision the
//! asynchronous caller carries out, which keeps the reuse, invalidation and
//! replay rules testable without a running agent.

use super::WarmOwner;
use agent_client_protocol_schema::v1::AvailableCommand;
use agent_client_protocol_schema::v1::{
    SessionConfigId, SessionConfigOption, SessionConfigOptionValue,
};
use ora_contracts::WarmSessionTarget;
use ora_domain::{AgentRef, SessionId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// How many provider sessions the pool keeps alive at once.
///
/// This is the only bound on live sessions; nothing releases one for having sat
/// unused. A warm session is by definition one the user never prompted — the
/// first prompt attaches it and removes it from the pool — so an idle entry
/// holds no conversation to reclaim, only an empty session on the agent side.
/// Bounding the count is enough to cap that, and unlike a deadline it is
/// enforceable at the one moment it can be exceeded: creating another session.
/// A deadline would need a timer to be honest, and would charge a user who
/// steps away and returns a full rebuild for no benefit.
const MAX_LIVE_ENTRIES: usize = 16;
/// How many entries — live or cold — the pool remembers at once.
///
/// Cold entries hold only an identifier and the options the user picked, so this
/// bound exists to cap unbounded growth rather than to reclaim meaningful memory.
const MAX_ENTRIES: usize = 64;

/// Identifies the Desktop or workflow surface a warm session belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct WarmKey {
    pub target: WarmSessionTarget,
    pub agent_ref: AgentRef,
    pub owner: WarmOwner,
}

/// A provider session that is currently registered on a live connection.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveSession {
    agent_session_id: String,
    /// The connection generation that created it; a rollover invalidates it.
    generation: u64,
}

/// One warm session, with or without a live provider session behind it.
#[derive(Debug, Clone)]
struct WarmEntry {
    session_id: SessionId,
    key: WarmKey,
    /// The directory the provider session was created against.
    ///
    /// Kept so a moved or recreated worktree is detected: the identity key names
    /// a Task, not a path, and reusing a session whose cwd drifted would send
    /// the agent to work in the wrong directory.
    cwd: PathBuf,
    /// `None` once the provider session was released; the entry survives so the
    /// identifier the client already holds keeps resolving.
    live: Option<LiveSession>,
    /// Options the user explicitly chose, replayed onto any rebuilt session.
    desired_config: HashMap<SessionConfigId, SessionConfigOptionValue>,
    config_options: Vec<SessionConfigOption>,
    /// The slash-command catalog the agent announced during the handshake.
    ///
    /// Captured here because ACP only sends it once, right after `session/new`,
    /// and nothing consumes the session's updates until it is attached. Keeping
    /// it lets the attach response describe the commands without a second
    /// handshake.
    available_commands: Vec<AvailableCommand>,
    last_used_at: i64,
    /// Set while an attach attempt holds this entry, so `lookup` skips it
    /// instead of handing its provider session to a second caller, and so a
    /// failed attach can release the reservation and try again from here
    /// instead of finding the entry already gone.
    reserved: bool,
}

/// What a caller must build when nothing usable sits behind an entry.
///
/// Shared by both ways a key is resolved so the two cannot drift on what a
/// handshake needs. `replay` carries the options the user had already chosen so
/// a rebuilt session comes back configured the way they left it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CreatePlan {
    pub session_id: SessionId,
    pub cwd: PathBuf,
    pub replay: Vec<(SessionConfigId, SessionConfigOptionValue)>,
}

/// What the caller must do to satisfy a warm-session request.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum WarmDecision {
    /// The existing provider session is current and may be served as-is.
    Ready {
        session_id: SessionId,
        agent_session_id: String,
        config_options: Vec<SessionConfigOption>,
    },
    /// A provider session must be created, after which `commit_created` records it.
    Create(CreatePlan),
}

/// What the caller must do to claim a warm session addressed by key.
///
/// Distinct from [`WarmDecision`] because claiming reserves what it resolves:
/// the caller is taking the provider session, not reading it. There is
/// deliberately no "unavailable" outcome — resolution skips reserved entries, so
/// whatever it lands on can always be reserved.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ClaimDecision {
    /// The entry was live and is now reserved for this caller.
    Held(AttachedWarm),
    /// The caller performs the handshake and installs it with
    /// `replace_and_reserve`, which reserves the entry in the same step.
    Create(CreatePlan),
}

/// A provider session that is no longer referenced and should be released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReleasedSession {
    pub agent_ref: AgentRef,
    pub agent_session_id: String,
    pub generation: u64,
}

/// A warm session promoted out of the pool to become a persisted Ora session.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct AttachedWarm {
    pub session_id: SessionId,
    pub agent_ref: AgentRef,
    pub agent_session_id: String,
    pub cwd: PathBuf,
    pub available_commands: Vec<AvailableCommand>,
    /// What the provider session being handed over currently offers.
    ///
    /// Carried because a caller that rebinds an existing Ora session has no
    /// other source for it: ACP reports configuration only while creating or
    /// loading a session, and the handshake that produced this one may have
    /// happened long before the claim. Attaching ignores it — that client read
    /// the same list from its own warm response.
    pub config_options: Vec<SessionConfigOption>,
}

/// Everything one completed `session/new` handshake produced.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CreatedProvider {
    pub agent_session_id: String,
    pub config_options: Vec<SessionConfigOption>,
    pub available_commands: Vec<AvailableCommand>,
}

/// What one attach attempt found when reserving a warm session.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Reservation {
    /// The warm session was live and is now held for this attach.
    Held(AttachedWarm),
    /// Nothing usable is behind this entry — it is cold, or its provider
    /// session was created against a different directory. The caller builds one
    /// and installs it with `replace_and_reserve`.
    NeedsRebuild,
    /// Another attach owns this warm session: it holds a reservation, or it
    /// committed and removed the entry while this caller was deciding.
    ///
    /// Rebuilding here would replace the provider session that attach is
    /// persisting against, leaving it with an identifier the agent has already
    /// dropped, so this caller fails instead.
    Unavailable,
}

/// Whether a freshly created provider session could be installed on its entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Install {
    /// The entry took it; any provider session it replaced is returned so the
    /// caller can release it.
    Accepted(Option<ReleasedSession>),
    /// The entry refused it: it disappeared while the handshake was in flight,
    /// or an attach reserved it and owns the session it already holds. The
    /// caller releases the session it just created, which now has no owner.
    Refused,
}

/// Where a `set_config_option` request should be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConfigTarget {
    /// The warm session is live; the agent can be asked directly.
    Live {
        agent_ref: AgentRef,
        agent_session_id: String,
    },
    /// The warm session is cold. The choice is recorded and replayed on rebuild,
    /// so the user's selection survives without paying for a session now.
    Deferred,
}

/// Owns every warm session and the rules that create, reuse and retire them.
#[derive(Debug, Default)]
pub(super) struct WarmPool {
    entries: Vec<WarmEntry>,
}

impl WarmPool {
    /// Resolves a warm-session request against the pool.
    ///
    /// `cwd` is re-derived by the caller on every request rather than cached, so
    /// a worktree that moved invalidates its entry here instead of silently
    /// addressing a stale path.
    pub(super) fn lookup(
        &mut self,
        key: &WarmKey,
        cwd: &Path,
        generation: u64,
        now: i64,
        next_session_id: impl FnOnce() -> SessionId,
    ) -> (WarmDecision, Option<ReleasedSession>) {
        let (index, released) = self.resolve(key, cwd, generation, now, next_session_id);
        let decision = match &self.entries[index].live {
            Some(live) => WarmDecision::Ready {
                session_id: self.entries[index].session_id.clone(),
                agent_session_id: live.agent_session_id.clone(),
                config_options: self.entries[index].config_options.clone(),
            },
            None => WarmDecision::Create(self.create_plan(index)),
        };
        (decision, released)
    }

    /// Resolves a key and reserves what it resolves to, in one critical section.
    ///
    /// This is `lookup` for a caller that is taking the provider session rather
    /// than reading it, and doing both here is the point: a caller that looked up
    /// an entry and then reserved it separately would leave a window in which
    /// another claim could take the session out from under it.
    ///
    /// Reserving always succeeds because `resolve` skips reserved entries — a key
    /// whose entry is already being claimed resolves to a fresh one instead. That
    /// is what keeps a second caller from failing rather than simply getting its
    /// own session, and it is why there is no "unavailable" outcome here.
    pub(super) fn lookup_and_reserve(
        &mut self,
        key: &WarmKey,
        cwd: &Path,
        generation: u64,
        now: i64,
        next_session_id: impl FnOnce() -> SessionId,
    ) -> (ClaimDecision, Option<ReleasedSession>) {
        let (index, released) = self.resolve(key, cwd, generation, now, next_session_id);
        let entry = &self.entries[index];
        let Some(live) = entry.live.as_ref() else {
            return (ClaimDecision::Create(self.create_plan(index)), released);
        };
        let attached = AttachedWarm {
            session_id: entry.session_id.clone(),
            agent_ref: entry.key.agent_ref.clone(),
            agent_session_id: live.agent_session_id.clone(),
            cwd: entry.cwd.clone(),
            available_commands: entry.available_commands.clone(),
            config_options: entry.config_options.clone(),
        };
        self.entries[index].reserved = true;
        (ClaimDecision::Held(attached), released)
    }

    /// Finds the entry that should serve one key, creating a cold one if none does.
    ///
    /// Reserved entries are skipped rather than shared, so a key whose entry is
    /// mid-claim resolves to a new one instead of handing two callers the same
    /// provider session. Any session retired on the way — by a moved directory or
    /// a connection rollover — is reported for the caller to release.
    fn resolve(
        &mut self,
        key: &WarmKey,
        cwd: &Path,
        generation: u64,
        now: i64,
        next_session_id: impl FnOnce() -> SessionId,
    ) -> (usize, Option<ReleasedSession>) {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| &entry.key == key && !entry.reserved)
        else {
            self.entries.push(WarmEntry {
                session_id: next_session_id(),
                key: key.clone(),
                cwd: cwd.to_path_buf(),
                live: None,
                desired_config: HashMap::new(),
                config_options: Vec::new(),
                available_commands: Vec::new(),
                last_used_at: now,
                reserved: false,
            });
            return (self.entries.len() - 1, None);
        };

        self.entries[index].last_used_at = now;
        let cwd_changed = self.entries[index].cwd != cwd;
        let stale_generation = self.entries[index]
            .live
            .as_ref()
            .is_some_and(|live| live.generation != generation);

        if cwd_changed || stale_generation {
            let released = self.release_live(index);
            // A directory change makes the recorded options meaningless only if
            // the agent reports different ones; keeping them lets the replay
            // restore the user's pick and the agent correct it if it cannot.
            self.entries[index].cwd = cwd.to_path_buf();
            return (index, released);
        }
        (index, None)
    }

    /// Records the provider session produced for a `Create` decision.
    ///
    /// Refuses a reserved entry rather than replacing what it holds: an attach
    /// is already persisting that provider session, and releasing it here would
    /// leave the persisted session addressing an identifier the agent dropped.
    /// Warming loses nothing by refusing — it reports the options it just read
    /// and the entry keeps the session the attach is taking ownership of.
    pub(super) fn commit_created(
        &mut self,
        session_id: &SessionId,
        created: CreatedProvider,
        generation: u64,
        now: i64,
    ) -> Install {
        let Some(index) = self.index_of(session_id) else {
            return Install::Refused;
        };
        if self.entries[index].reserved {
            return Install::Refused;
        }
        let released = self.release_live(index);
        let entry = &mut self.entries[index];
        entry.live = Some(LiveSession {
            agent_session_id: created.agent_session_id,
            generation,
        });
        entry.config_options = created.config_options;
        entry.available_commands = created.available_commands;
        entry.last_used_at = now;
        Install::Accepted(released)
    }

    /// Reports where a configuration change for one warm session must be sent.
    pub(super) fn config_target(
        &mut self,
        session_id: &SessionId,
        now: i64,
    ) -> Option<ConfigTarget> {
        let index = self.index_of(session_id)?;
        self.entries[index].last_used_at = now;
        let entry = &self.entries[index];
        Some(match &entry.live {
            Some(live) => ConfigTarget::Live {
                agent_ref: entry.key.agent_ref.clone(),
                agent_session_id: live.agent_session_id.clone(),
            },
            None => ConfigTarget::Deferred,
        })
    }

    /// Records a configuration choice so it survives a later rebuild.
    ///
    /// `config_options` is the agent's own report when one is available. It is
    /// authoritative: an agent that rejected or adjusted the request describes
    /// the outcome here, and the client renders that rather than the request.
    pub(super) fn record_config(
        &mut self,
        session_id: &SessionId,
        config_id: SessionConfigId,
        value: SessionConfigOptionValue,
        config_options: Option<Vec<SessionConfigOption>>,
    ) -> Vec<SessionConfigOption> {
        let Some(index) = self.index_of(session_id) else {
            return config_options.unwrap_or_default();
        };
        let entry = &mut self.entries[index];
        entry.desired_config.insert(config_id, value);
        if let Some(config_options) = config_options {
            entry.config_options = config_options;
        }
        entry.config_options.clone()
    }

    /// Reserves one warm session for an attach attempt, without removing it yet.
    ///
    /// The entry stays in the pool — invisible to `lookup` while reserved — so a
    /// caller whose later persistence steps fail can call `release_reservation`
    /// and find the same provider session still here to retry with, instead of
    /// discovering it gone and stranding the client's id. `commit_attach`
    /// finalizes the removal once persistence actually succeeds.
    ///
    /// `cwd` is the attaching Task's authoritative directory. A session created
    /// against a different one is reported as needing a rebuild rather than
    /// reserved, because the alternative is an agent quietly working in the
    /// wrong directory.
    ///
    /// A cold entry needs a rebuild; one another attach already owns is refused
    /// outright. Distinguishing the two is what keeps a second attach of the
    /// same identifier from rebuilding over the first one's provider session,
    /// which is indistinguishable from a cold entry unless it is named.
    pub(super) fn reserve_for_attach(&mut self, session_id: &SessionId, cwd: &Path) -> Reservation {
        let Some(index) = self.index_of(session_id) else {
            return Reservation::Unavailable;
        };
        if self.entries[index].reserved {
            return Reservation::Unavailable;
        }
        let entry = &self.entries[index];
        let Some(live) = entry.live.as_ref().filter(|_| entry.cwd == cwd) else {
            return Reservation::NeedsRebuild;
        };
        let attached = AttachedWarm {
            session_id: entry.session_id.clone(),
            agent_ref: entry.key.agent_ref.clone(),
            agent_session_id: live.agent_session_id.clone(),
            cwd: entry.cwd.clone(),
            available_commands: entry.available_commands.clone(),
            config_options: entry.config_options.clone(),
        };
        self.entries[index].reserved = true;
        Reservation::Held(attached)
    }

    /// Finalizes an attach that succeeded: the provider session now belongs to
    /// the persisted Ora session, so the pool no longer needs to track it.
    pub(super) fn commit_attach(&mut self, session_id: &SessionId) {
        if let Some(index) = self.index_of(session_id) {
            self.entries.remove(index);
        }
    }

    /// Reverts a reservation after the caller's persistence steps failed.
    ///
    /// The provider session and its entry are left exactly as they were, so the
    /// next attempt for this same id finds a usable warm session again instead
    /// of `warm_session_not_found`.
    pub(super) fn release_reservation(&mut self, session_id: &SessionId) {
        if let Some(index) = self.index_of(session_id) {
            self.entries[index].reserved = false;
        }
    }

    /// Replaces a superseded entry's provider session with a freshly rebuilt
    /// one and reserves it for attach, mirroring `reserve_for_attach` for the
    /// case where the old session could not be reused.
    ///
    /// Refuses an entry that disappeared or that another attach reserved while
    /// this rebuild was in flight. The window is real: the caller performs a
    /// `session/new` between deciding to rebuild and arriving here, which is
    /// long enough for a second attach of the same identifier to reserve the
    /// entry. Installing anyway would release the provider session that attach
    /// is persisting against.
    pub(super) fn replace_and_reserve(
        &mut self,
        session_id: &SessionId,
        cwd: PathBuf,
        created: CreatedProvider,
        generation: u64,
        now: i64,
    ) -> Install {
        let Some(index) = self.index_of(session_id) else {
            return Install::Refused;
        };
        if self.entries[index].reserved {
            return Install::Refused;
        }
        let released = self.release_live(index);
        let entry = &mut self.entries[index];
        entry.cwd = cwd;
        entry.live = Some(LiveSession {
            agent_session_id: created.agent_session_id,
            generation,
        });
        entry.config_options = created.config_options;
        entry.available_commands = created.available_commands;
        entry.reserved = true;
        entry.last_used_at = now;
        Install::Accepted(released)
    }

    /// Returns what a cold or missing entry needs in order to be rebuilt.
    pub(super) fn rebuild_plan(&self, session_id: &SessionId) -> Option<RebuildPlan> {
        let index = self.index_of(session_id)?;
        let entry = &self.entries[index];
        Some(RebuildPlan {
            agent_ref: entry.key.agent_ref.clone(),
            cwd: entry.cwd.clone(),
            replay: entry.desired_config.clone().into_iter().collect(),
        })
    }

    /// Releases every provider session belonging to a superseded connection generation.
    ///
    /// A CLI that crashed and restarted leaves behind identifiers that no longer
    /// resolve. The entries survive as cold so the identifiers clients hold keep
    /// working; only the dead provider sessions are dropped.
    pub(super) fn invalidate_generation(&mut self, agent_ref: &AgentRef, generation: u64) {
        for entry in &mut self.entries {
            if entry.key.agent_ref == *agent_ref
                && entry
                    .live
                    .as_ref()
                    .is_some_and(|live| live.generation != generation)
            {
                entry.live = None;
            }
        }
    }

    /// Drops every entry for a chat surface that no longer exists, reporting
    /// their provider sessions for release.
    ///
    /// A warm session is only ever reached again through its target, so an entry
    /// whose workspace or project was deleted is unreachable: no lookup can reuse it,
    /// and because reuse is what triggers a rebuild, nothing retires it either.
    /// The count bounds would push it out eventually, but only once enough new
    /// surfaces are opened to do so — which for a user who deletes a Task and
    /// carries on in a handful of others may be never. Deleting the target is
    /// the last moment its provider session can be reclaimed.
    ///
    /// Reserved entries are kept for the reason [`WarmPool::evict`] keeps them:
    /// an attach already holds the provider session, and releasing it would
    /// leave that attach addressing an identifier the agent dropped. Losing the
    /// entry costs nothing — the attach is taking ownership of the session — but
    /// losing the session under it is exactly the failure the reservation exists
    /// to prevent.
    pub(super) fn discard_targets(
        &mut self,
        targets: &[WarmSessionTarget],
    ) -> Vec<ReleasedSession> {
        let mut released = Vec::new();
        let mut index = 0;
        while index < self.entries.len() {
            let entry = &self.entries[index];
            if entry.reserved || !targets.contains(&entry.key.target) {
                index += 1;
                continue;
            }
            released.extend(self.release_live(index));
            self.entries.remove(index);
        }
        released
    }

    /// Retires the least recently used sessions once the pool's bounds are passed.
    ///
    /// Both bounds are pure counts, so this is driven by the one operation that
    /// can exceed them — installing a newly created provider session — and needs
    /// no notion of the current time.
    ///
    /// Reserved entries are exempt from every bound here. An attach already
    /// holds their provider session id and is racing to persist an Ora session
    /// against it, so releasing it would delete that session out from under the
    /// row being written, and removing the entry would leave a failed attach
    /// with nothing to fall back to. Eviction runs on whichever chat surface
    /// created a session, so nothing stops it from landing inside an unrelated
    /// surface's attach window.
    ///
    /// Both bounds are therefore measured against what is actually evictable
    /// rather than against every entry, which lets the pool briefly exceed them
    /// by the number of concurrent attaches. Counting reservations in would only
    /// mean retiring an extra session a user is still looking at to make room
    /// for one that is leaving the pool anyway, and — when reservations
    /// outnumber the bound — deleting every remaining entry in a doomed attempt
    /// to reach a total no amount of eviction can reach.
    pub(super) fn evict(&mut self) -> Vec<ReleasedSession> {
        let mut released = Vec::new();
        let mut live: Vec<usize> = (0..self.entries.len())
            .filter(|index| self.entries[*index].live.is_some() && !self.entries[*index].reserved)
            .collect();
        live.sort_by_key(|index| self.entries[*index].last_used_at);
        let over_capacity = live.len().saturating_sub(MAX_LIVE_ENTRIES);
        for index in live.into_iter().take(over_capacity) {
            released.extend(self.release_live(index));
        }

        if self.entries.len() > MAX_ENTRIES {
            let mut order: Vec<usize> = (0..self.entries.len())
                .filter(|index| !self.entries[*index].reserved)
                .collect();
            order.sort_by_key(|index| self.entries[*index].last_used_at);
            let over_bound = order.len().saturating_sub(MAX_ENTRIES);
            let mut doomed: Vec<usize> = order.into_iter().take(over_bound).collect();
            doomed.sort_unstable_by(|left, right| right.cmp(left));
            for index in doomed {
                released.extend(self.release_live(index));
                self.entries.remove(index);
            }
        }
        released
    }

    /// Builds what an entry needs handshaken, carrying its recorded choices.
    fn create_plan(&self, index: usize) -> CreatePlan {
        let entry = &self.entries[index];
        CreatePlan {
            session_id: entry.session_id.clone(),
            cwd: entry.cwd.clone(),
            replay: entry.desired_config.clone().into_iter().collect(),
        }
    }

    /// Detaches an entry's provider session and reports it for release.
    fn release_live(&mut self, index: usize) -> Option<ReleasedSession> {
        let agent_ref = self.entries[index].key.agent_ref.clone();
        self.entries[index].live.take().map(|live| ReleasedSession {
            agent_ref,
            agent_session_id: live.agent_session_id,
            generation: live.generation,
        })
    }

    fn index_of(&self, session_id: &SessionId) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| &entry.session_id == session_id)
    }
}

/// What a cold warm session needs before it can serve a prompt.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct RebuildPlan {
    pub agent_ref: AgentRef,
    pub cwd: PathBuf,
    pub replay: Vec<(SessionConfigId, SessionConfigOptionValue)>,
}

#[cfg(test)]
mod tests {
    use super::{
        AttachedWarm, ClaimDecision, ConfigTarget, CreatePlan, CreatedProvider, Install,
        MAX_ENTRIES, MAX_LIVE_ENTRIES, RebuildPlan, ReleasedSession, Reservation, WarmDecision,
        WarmKey, WarmPool,
    };
    use crate::agent_runtime::WarmOwner;
    use agent_client_protocol_schema::v1::{
        SessionConfigOption, SessionConfigOptionValue, SessionConfigSelectOption,
    };
    use ora_contracts::WarmSessionTarget;
    use ora_domain::{AgentCli, SessionId};
    use pretty_assertions::assert_eq;
    use std::path::{Path, PathBuf};

    const GENERATION: u64 = 1;

    /// Builds a key for one workspace-scoped warm-session owner.
    fn key(workspace_id: &str, owner_name: &str) -> WarmKey {
        let owner = match owner_name {
            "interactive" | "client-1" => WarmOwner::Interactive,
            "workflow-node" | "client-2" => WarmOwner::WorkflowNode {
                run_id: "run-1".to_string(),
                node_id: "node-1".to_string(),
            },
            other => panic!("unknown warm owner {other}"),
        };
        WarmKey {
            target: WarmSessionTarget::Workspace {
                workspace_id: workspace_id.to_string(),
            },
            agent_ref: AgentCli::Nga.agent_ref(),
            owner,
        }
    }

    fn model_options(current: &str) -> Vec<SessionConfigOption> {
        vec![SessionConfigOption::select(
            "model",
            "Model",
            current.to_string(),
            vec![
                SessionConfigSelectOption::new("fast", "Fast"),
                SessionConfigSelectOption::new("smart", "Smart"),
            ],
        )]
    }

    /// Creates one live warm session and returns its Ora session id.
    fn warm(pool: &mut WarmPool, key: &WarmKey, cwd: &Path, now: i64, id: &str) -> SessionId {
        let (decision, _) = pool.lookup(key, cwd, GENERATION, now, || SessionId::new(id));
        let WarmDecision::Create(CreatePlan { session_id, .. }) = decision else {
            panic!("expected a create decision for a cold key");
        };
        pool.commit_created(
            &session_id,
            CreatedProvider {
                agent_session_id: format!("agent-{id}"),
                config_options: model_options("fast"),
                available_commands: Vec::new(),
            },
            GENERATION,
            now,
        );
        session_id
    }

    /// Verifies a first request creates and a second reuses the same provider session.
    #[test]
    fn reuses_a_live_warm_session_for_the_same_surface() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");

        let (decision, released) = pool.lookup(&key, Path::new("/repo"), GENERATION, 10, || {
            SessionId::new("unused")
        });

        assert_eq!(
            (decision, released),
            (
                WarmDecision::Ready {
                    session_id,
                    agent_session_id: "agent-session-1".to_string(),
                    config_options: model_options("fast"),
                },
                None,
            )
        );
    }

    /// Verifies interactive and workflow owners on one selection never share a provider session.
    #[test]
    fn isolates_warm_sessions_per_owner() {
        let mut pool = WarmPool::default();
        let first = warm(
            &mut pool,
            &key("workspace-1", "client-1"),
            Path::new("/repo"),
            0,
            "session-1",
        );

        let (decision, released) = pool.lookup(
            &key("workspace-1", "client-2"),
            Path::new("/repo"),
            GENERATION,
            0,
            || SessionId::new("session-2"),
        );

        assert_eq!(
            (decision, released),
            (
                WarmDecision::Create(CreatePlan {
                    session_id: SessionId::new("session-2"),
                    cwd: PathBuf::from("/repo"),
                    replay: vec![],
                }),
                None,
            )
        );
        assert_eq!(first, SessionId::new("session-1"));
    }

    /// Verifies a worktree that moved retires the stale session instead of reusing it.
    #[test]
    fn rebuilds_when_the_working_directory_changed() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo/old"), 0, "session-1");

        let (decision, released) = pool.lookup(&key, Path::new("/repo/new"), GENERATION, 5, || {
            SessionId::new("unused")
        });

        assert_eq!(
            (decision, released),
            (
                WarmDecision::Create(CreatePlan {
                    session_id,
                    cwd: PathBuf::from("/repo/new"),
                    replay: vec![],
                }),
                Some(ReleasedSession {
                    agent_ref: AgentCli::Nga.agent_ref(),
                    agent_session_id: "agent-session-1".to_string(),
                    generation: GENERATION,
                }),
            )
        );
    }

    /// Verifies a CLI restart leaves the identifier usable while dropping the dead session.
    #[test]
    fn keeps_the_identifier_after_a_connection_rollover() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");

        pool.invalidate_generation(&AgentCli::Nga.agent_ref(), GENERATION + 1);
        let (decision, released) = pool.lookup(&key, Path::new("/repo"), GENERATION + 1, 5, || {
            SessionId::new("unused")
        });

        assert_eq!(
            (decision, released),
            (
                WarmDecision::Create(CreatePlan {
                    session_id,
                    cwd: PathBuf::from("/repo"),
                    replay: vec![],
                }),
                None,
            )
        );
    }

    /// Verifies a recorded model choice is replayed onto a rebuilt session.
    #[test]
    fn replays_the_recorded_configuration_after_a_rebuild() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");
        pool.record_config(
            &session_id,
            "model".into(),
            SessionConfigOptionValue::value_id("smart"),
            Some(model_options("smart")),
        );

        pool.invalidate_generation(&AgentCli::Nga.agent_ref(), GENERATION + 1);
        let (decision, _) = pool.lookup(&key, Path::new("/repo"), GENERATION + 1, 5, || {
            SessionId::new("unused")
        });

        assert_eq!(
            decision,
            WarmDecision::Create(CreatePlan {
                session_id,
                cwd: PathBuf::from("/repo"),
                replay: vec![("model".into(), SessionConfigOptionValue::value_id("smart"),)],
            })
        );
    }

    /// Verifies a cold session records choices without paying for a provider session.
    #[test]
    fn defers_configuration_for_a_cold_session() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");
        pool.invalidate_generation(&AgentCli::Nga.agent_ref(), GENERATION + 1);

        assert_eq!(
            pool.config_target(&session_id, 5),
            Some(ConfigTarget::Deferred)
        );
    }

    /// Verifies a session that is merely unused keeps its provider session, so
    /// returning to a chat left open does not pay for a rebuild.
    #[test]
    fn keeps_an_unused_session_within_the_live_bound() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");

        let released = pool.evict();

        assert_eq!(
            (released, pool.config_target(&session_id, i64::MAX)),
            (
                vec![],
                Some(ConfigTarget::Live {
                    agent_ref: AgentCli::Nga.agent_ref(),
                    agent_session_id: "agent-session-1".to_string(),
                }),
            )
        );
    }

    /// Verifies the oldest provider sessions are released once the live bound is passed.
    #[test]
    fn releases_the_least_recently_used_session_over_capacity() {
        let mut pool = WarmPool::default();
        for index in 0..=MAX_LIVE_ENTRIES {
            warm(
                &mut pool,
                &key(&format!("task-{index}"), "client-1"),
                Path::new("/repo"),
                index as i64,
                &format!("session-{index}"),
            );
        }

        assert_eq!(
            pool.evict(),
            vec![ReleasedSession {
                agent_ref: AgentCli::Nga.agent_ref(),
                agent_session_id: "agent-session-0".to_string(),
                generation: GENERATION,
            }]
        );
    }

    /// Verifies the live bound skips the session being attached even when it is
    /// the least recently used one, and releases the next oldest in its place.
    #[test]
    fn keeps_the_session_being_attached_over_capacity() {
        let mut pool = WarmPool::default();
        let session_ids: Vec<SessionId> = (0..=MAX_LIVE_ENTRIES + 1)
            .map(|index| {
                warm(
                    &mut pool,
                    &key(&format!("task-{index}"), "client-1"),
                    Path::new("/repo"),
                    index as i64,
                    &format!("session-{index}"),
                )
            })
            .collect();
        pool.reserve_for_attach(&session_ids[0], Path::new("/repo"));

        assert_eq!(
            pool.evict(),
            vec![ReleasedSession {
                agent_ref: AgentCli::Nga.agent_ref(),
                agent_session_id: "agent-session-1".to_string(),
                generation: GENERATION,
            }]
        );
    }

    /// Verifies the entry bound never drops the entry being attached, so an
    /// attach that fails afterwards still finds its warm session to retry with
    /// instead of a stranded id.
    #[test]
    fn keeps_the_entry_being_attached_within_the_entry_bound() {
        let mut pool = WarmPool::default();
        let reserved = warm(
            &mut pool,
            &key("task-0", "client-1"),
            Path::new("/repo"),
            0,
            "session-0",
        );
        pool.reserve_for_attach(&reserved, Path::new("/repo"));
        // A `lookup` with no matching commit leaves a cold entry, which is the
        // cheapest way to push the evictable population past the entry bound.
        for index in 1..=MAX_ENTRIES + 1 {
            pool.lookup(
                &key(&format!("task-{index}"), "client-1"),
                Path::new("/repo"),
                GENERATION,
                index as i64,
                || SessionId::new(format!("session-{index}")),
            );
        }

        pool.evict();

        assert_eq!(
            (
                pool.rebuild_plan(&reserved),
                pool.rebuild_plan(&SessionId::new("session-1")),
            ),
            (
                Some(RebuildPlan {
                    agent_ref: AgentCli::Nga.agent_ref(),
                    cwd: PathBuf::from("/repo"),
                    replay: vec![],
                }),
                None,
            )
        );
    }

    /// Verifies reservations outnumbering the entry bound cannot turn eviction
    /// into a purge: the bound is unreachable while they are held, and chasing
    /// it would delete every entry that is actually in use.
    #[test]
    fn keeps_usable_entries_when_reservations_outnumber_the_entry_bound() {
        let mut pool = WarmPool::default();
        let reserved: Vec<SessionId> = (0..=MAX_ENTRIES)
            .map(|index| {
                let session_id = warm(
                    &mut pool,
                    &key(&format!("task-{index}"), "client-1"),
                    Path::new("/repo"),
                    index as i64,
                    &format!("session-{index}"),
                );
                pool.reserve_for_attach(&session_id, Path::new("/repo"));
                session_id
            })
            .collect();
        let usable = warm(
            &mut pool,
            &key("task-usable", "client-1"),
            Path::new("/repo"),
            MAX_ENTRIES as i64 + 1,
            "session-usable",
        );

        pool.evict();

        assert_eq!(
            (
                pool.rebuild_plan(&usable).is_some(),
                pool.rebuild_plan(&reserved[0]).is_some(),
            ),
            (true, true)
        );
    }

    /// Verifies a deleted target takes its warm sessions with it, across every
    /// owner that had one open, while leaving other surfaces untouched.
    #[test]
    fn discards_every_warm_session_for_a_deleted_target() {
        let mut pool = WarmPool::default();
        let doomed_first = warm(
            &mut pool,
            &key("workspace-1", "client-1"),
            Path::new("/repo"),
            0,
            "session-1",
        );
        let doomed_second = warm(
            &mut pool,
            &key("workspace-1", "client-2"),
            Path::new("/repo"),
            1,
            "session-2",
        );
        let survivor = warm(
            &mut pool,
            &key("workspace-2", "client-1"),
            Path::new("/repo"),
            2,
            "session-3",
        );

        let released = pool.discard_targets(&[WarmSessionTarget::Workspace {
            workspace_id: "workspace-1".to_string(),
        }]);

        assert_eq!(
            (
                released,
                pool.rebuild_plan(&doomed_first),
                pool.rebuild_plan(&doomed_second),
                pool.rebuild_plan(&survivor).is_some(),
            ),
            (
                vec![
                    ReleasedSession {
                        agent_ref: AgentCli::Nga.agent_ref(),
                        agent_session_id: "agent-session-1".to_string(),
                        generation: GENERATION,
                    },
                    ReleasedSession {
                        agent_ref: AgentCli::Nga.agent_ref(),
                        agent_session_id: "agent-session-2".to_string(),
                        generation: GENERATION,
                    },
                ],
                None,
                None,
                true,
            )
        );
    }

    /// Verifies a deleted target leaves an in-flight attach alone, so the
    /// provider session it is persisting is not deleted underneath it.
    #[test]
    fn keeps_the_session_being_attached_when_its_target_is_deleted() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");
        pool.reserve_for_attach(&session_id, Path::new("/repo"));

        let released = pool.discard_targets(&[WarmSessionTarget::Workspace {
            workspace_id: "workspace-1".to_string(),
        }]);

        assert_eq!(
            (released, pool.config_target(&session_id, 5)),
            (
                vec![],
                Some(ConfigTarget::Live {
                    agent_ref: AgentCli::Nga.agent_ref(),
                    agent_session_id: "agent-session-1".to_string(),
                }),
            )
        );
    }

    /// Verifies a reserved entry is hidden from `lookup` so a concurrent surface
    /// warms a fresh session instead of sharing the one being attached.
    #[test]
    fn hides_a_reserved_entry_from_lookup() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");

        let attached = pool.reserve_for_attach(&session_id, Path::new("/repo"));
        let (decision, _) = pool.lookup(&key, Path::new("/repo"), GENERATION, 5, || {
            SessionId::new("session-2")
        });

        assert_eq!(
            (attached, decision),
            (
                Reservation::Held(AttachedWarm {
                    session_id,
                    agent_ref: AgentCli::Nga.agent_ref(),
                    agent_session_id: "agent-session-1".to_string(),
                    cwd: PathBuf::from("/repo"),
                    available_commands: vec![],
                    config_options: model_options("fast"),
                }),
                WarmDecision::Create(CreatePlan {
                    session_id: SessionId::new("session-2"),
                    cwd: PathBuf::from("/repo"),
                    replay: vec![],
                }),
            )
        );
    }

    /// Verifies a second reservation attempt on the same entry is refused.
    #[test]
    fn refuses_a_second_reservation_of_the_same_entry() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");

        pool.reserve_for_attach(&session_id, Path::new("/repo"));

        assert_eq!(
            pool.reserve_for_attach(&session_id, Path::new("/repo")),
            Reservation::Unavailable
        );
    }

    /// Verifies claiming a live key reserves it in the same step and reports
    /// what that provider session offers — the only place a caller rebinding an
    /// existing session can learn the incoming agent's model list.
    #[test]
    fn reserves_a_live_entry_when_claiming_by_key() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");

        let (decision, released) =
            pool.lookup_and_reserve(&key, Path::new("/repo"), GENERATION, 5, || {
                SessionId::new("unused")
            });

        assert_eq!(
            (
                decision,
                released,
                pool.rebuild_plan(&SessionId::new("unused"))
            ),
            (
                ClaimDecision::Held(AttachedWarm {
                    session_id,
                    agent_ref: AgentCli::Nga.agent_ref(),
                    agent_session_id: "agent-session-1".to_string(),
                    cwd: PathBuf::from("/repo"),
                    available_commands: vec![],
                    config_options: model_options("fast"),
                }),
                None,
                // Nothing was minted: the live entry served the claim.
                None,
            )
        );
    }

    /// Verifies a claim whose entry is already being attached resolves to a
    /// fresh one rather than failing. Switching a session onto a CLI must not
    /// depend on whether an unrelated surface under the same Task is mid-attach.
    #[test]
    fn claims_a_fresh_entry_when_the_key_is_already_reserved() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let reserved = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");
        pool.reserve_for_attach(&reserved, Path::new("/repo"));

        let (decision, released) =
            pool.lookup_and_reserve(&key, Path::new("/repo"), GENERATION, 5, || {
                SessionId::new("session-2")
            });

        assert_eq!(
            (decision, released),
            (
                ClaimDecision::Create(CreatePlan {
                    session_id: SessionId::new("session-2"),
                    cwd: PathBuf::from("/repo"),
                    replay: vec![],
                }),
                None,
            )
        );
    }

    /// Verifies a claim whose provider session died with its connection reports
    /// the rebuild carrying the user's recorded choice, so a model picked for
    /// the incoming agent survives the handshake the claim has to pay for.
    #[test]
    fn carries_recorded_choices_when_claiming_a_cold_entry() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");
        pool.record_config(
            &session_id,
            "model".into(),
            SessionConfigOptionValue::value_id("smart"),
            Some(model_options("smart")),
        );
        pool.invalidate_generation(&AgentCli::Nga.agent_ref(), GENERATION + 1);

        let (decision, _) =
            pool.lookup_and_reserve(&key, Path::new("/repo"), GENERATION + 1, 5, || {
                SessionId::new("unused")
            });

        assert_eq!(
            decision,
            ClaimDecision::Create(CreatePlan {
                session_id,
                cwd: PathBuf::from("/repo"),
                replay: vec![("model".into(), SessionConfigOptionValue::value_id("smart"))],
            })
        );
    }

    /// Verifies a rebuild cannot install over a reservation another attach
    /// holds. The `session/new` between deciding to rebuild and arriving here
    /// is exactly the window a second attach of the same identifier needs.
    #[test]
    fn refuses_a_rebuild_that_would_replace_a_held_reservation() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");
        pool.reserve_for_attach(&session_id, Path::new("/repo"));

        let installed = pool.replace_and_reserve(
            &session_id,
            PathBuf::from("/repo"),
            CreatedProvider {
                agent_session_id: "agent-session-2".to_string(),
                config_options: model_options("fast"),
                available_commands: Vec::new(),
            },
            GENERATION,
            5,
        );

        assert_eq!(
            (installed, pool.config_target(&session_id, 5)),
            (
                Install::Refused,
                Some(ConfigTarget::Live {
                    agent_ref: AgentCli::Nga.agent_ref(),
                    agent_session_id: "agent-session-1".to_string(),
                }),
            )
        );
    }

    /// Verifies warming cannot install over a reservation either, so an attach
    /// racing a second `warm` of the same identifier keeps the provider session
    /// it is persisting rather than having it swapped underneath.
    #[test]
    fn refuses_a_created_session_that_would_replace_a_held_reservation() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");
        pool.reserve_for_attach(&session_id, Path::new("/repo"));

        let installed = pool.commit_created(
            &session_id,
            CreatedProvider {
                agent_session_id: "agent-session-2".to_string(),
                config_options: model_options("fast"),
                available_commands: Vec::new(),
            },
            GENERATION,
            5,
        );

        assert_eq!(
            (installed, pool.config_target(&session_id, 5)),
            (
                Install::Refused,
                Some(ConfigTarget::Live {
                    agent_ref: AgentCli::Nga.agent_ref(),
                    agent_session_id: "agent-session-1".to_string(),
                }),
            )
        );
    }

    /// Verifies a session created against another directory reports a rebuild
    /// instead of being reserved, and is left unreserved for that rebuild to
    /// claim rather than held by the attempt that rejected it.
    #[test]
    fn needs_a_rebuild_when_the_attaching_directory_differs() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo/old"), 0, "session-1");

        let reservation = pool.reserve_for_attach(&session_id, Path::new("/repo/new"));

        assert_eq!(
            (
                reservation,
                pool.reserve_for_attach(&session_id, Path::new("/repo/old")),
            ),
            (
                Reservation::NeedsRebuild,
                Reservation::Held(AttachedWarm {
                    session_id,
                    agent_ref: AgentCli::Nga.agent_ref(),
                    agent_session_id: "agent-session-1".to_string(),
                    cwd: PathBuf::from("/repo/old"),
                    available_commands: vec![],
                    config_options: model_options("fast"),
                }),
            )
        );
    }

    /// Verifies an identifier whose entry is gone is refused rather than
    /// rebuilt: the attach that removed it has already persisted the session.
    #[test]
    fn refuses_an_identifier_whose_entry_was_committed_away() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");
        pool.reserve_for_attach(&session_id, Path::new("/repo"));
        pool.commit_attach(&session_id);

        assert_eq!(
            pool.reserve_for_attach(&session_id, Path::new("/repo")),
            Reservation::Unavailable
        );
    }

    /// Verifies a committed attach removes the entry so the id is no longer warm.
    #[test]
    fn removes_the_entry_once_the_attach_is_committed() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");
        pool.reserve_for_attach(&session_id, Path::new("/repo"));

        pool.commit_attach(&session_id);

        assert_eq!(pool.rebuild_plan(&session_id), None);
    }

    /// Verifies a released reservation leaves the same provider session usable,
    /// so a failed attach can retry instead of finding the id gone.
    #[test]
    fn reuses_the_entry_after_a_reservation_is_released() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");
        pool.reserve_for_attach(&session_id, Path::new("/repo"));

        pool.release_reservation(&session_id);

        assert_eq!(
            pool.reserve_for_attach(&session_id, Path::new("/repo")),
            Reservation::Held(AttachedWarm {
                session_id,
                agent_ref: AgentCli::Nga.agent_ref(),
                agent_session_id: "agent-session-1".to_string(),
                cwd: PathBuf::from("/repo"),
                available_commands: vec![],
                config_options: model_options("fast"),
            })
        );
    }

    /// Verifies a cold session keeps everything needed to rebuild before attaching.
    #[test]
    fn reports_a_rebuild_plan_for_a_cold_session() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");
        pool.record_config(
            &session_id,
            "model".into(),
            SessionConfigOptionValue::value_id("smart"),
            None,
        );
        pool.invalidate_generation(&AgentCli::Nga.agent_ref(), GENERATION + 1);

        assert_eq!(
            (
                pool.reserve_for_attach(&session_id, Path::new("/repo")),
                pool.rebuild_plan(&session_id)
            ),
            (
                Reservation::NeedsRebuild,
                Some(RebuildPlan {
                    agent_ref: AgentCli::Nga.agent_ref(),
                    cwd: PathBuf::from("/repo"),
                    replay: vec![("model".into(), SessionConfigOptionValue::value_id("smart"),)],
                }),
            )
        );
    }

    /// Verifies a rebuilt session replaces the superseded one and stays
    /// reserved, with the old provider session reported for release.
    #[test]
    fn replaces_and_reserves_the_entry_on_rebuild() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo/old"), 0, "session-1");

        let released = pool.replace_and_reserve(
            &session_id,
            PathBuf::from("/repo/new"),
            CreatedProvider {
                agent_session_id: "agent-session-2".to_string(),
                config_options: model_options("fast"),
                available_commands: Vec::new(),
            },
            GENERATION,
            5,
        );

        assert_eq!(
            (
                released,
                pool.reserve_for_attach(&session_id, Path::new("/repo"))
            ),
            (
                Install::Accepted(Some(ReleasedSession {
                    agent_ref: AgentCli::Nga.agent_ref(),
                    agent_session_id: "agent-session-1".to_string(),
                    generation: GENERATION,
                })),
                // Already reserved by `replace_and_reserve`, so a second
                // reservation attempt is refused.
                Reservation::Unavailable,
            )
        );
    }
}
