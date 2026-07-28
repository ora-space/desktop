//! Bookkeeping for provider sessions created before the user's first prompt.
//!
//! ACP only reports a session's configuration options — the model selector among
//! them — as part of `session/new`. Ora therefore creates a session as soon as a
//! chat surface opens, so a model can be chosen before anything is sent. This
//! module owns which of those sessions exist, which may be reused, and which
//! must be torn down. It performs no I/O: every method returns a decision the
//! asynchronous caller carries out, which keeps the reuse, invalidation and
//! replay rules testable without a running agent.

use ora_contracts::WarmSessionTarget;
use ora_contracts::acp::session_config_options::{
    SessionConfigId, SessionConfigOption, SessionConfigOptionValue,
};
use ora_contracts::acp::slash_command::AvailableCommand;
use ora_domain::{AgentCli, SessionId};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// How long a warm session may sit unused before its provider session is released.
const IDLE_TTL_MILLIS: i64 = 10 * 60 * 1000;
/// How many provider sessions the pool keeps alive at once.
const MAX_LIVE_ENTRIES: usize = 8;
/// How many entries — live or cold — the pool remembers at once.
///
/// Cold entries hold only an identifier and the options the user picked, so this
/// bound exists to cap unbounded growth rather than to reclaim meaningful memory.
const MAX_ENTRIES: usize = 64;

/// Identifies the chat surface a warm session belongs to.
///
/// `client_id` is part of the identity because one backend can serve several
/// clients. Two browser tabs showing the same selection must not receive the
/// same provider session, or the first tab to attach would take the other's
/// conversation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct WarmKey {
    pub target: WarmSessionTarget,
    pub agent_cli: AgentCli,
    pub client_id: String,
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
    ///
    /// `replay` carries the options the user had already chosen so a rebuilt
    /// session comes back configured the way they left it.
    Create {
        session_id: SessionId,
        cwd: PathBuf,
        replay: Vec<(SessionConfigId, SessionConfigOptionValue)>,
    },
}

/// A provider session that is no longer referenced and should be released.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReleasedSession {
    pub agent_cli: AgentCli,
    pub agent_session_id: String,
    pub generation: u64,
}

/// A warm session promoted out of the pool to become a persisted Ora session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AttachedWarm {
    pub session_id: SessionId,
    pub agent_cli: AgentCli,
    pub agent_session_id: String,
    pub cwd: PathBuf,
    pub available_commands: Vec<AvailableCommand>,
}

/// Everything one completed `session/new` handshake produced.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct CreatedProvider {
    pub agent_session_id: String,
    pub config_options: Vec<SessionConfigOption>,
    pub available_commands: Vec<AvailableCommand>,
}

/// Where a `set_config_option` request should be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConfigTarget {
    /// The warm session is live; the agent can be asked directly.
    Live {
        agent_cli: AgentCli,
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
        let Some(index) = self.entries.iter().position(|entry| &entry.key == key) else {
            let session_id = next_session_id();
            self.entries.push(WarmEntry {
                session_id: session_id.clone(),
                key: key.clone(),
                cwd: cwd.to_path_buf(),
                live: None,
                desired_config: HashMap::new(),
                config_options: Vec::new(),
                available_commands: Vec::new(),
                last_used_at: now,
            });
            return (
                WarmDecision::Create {
                    session_id,
                    cwd: cwd.to_path_buf(),
                    replay: Vec::new(),
                },
                None,
            );
        };

        self.entries[index].last_used_at = now;
        let cwd_changed = self.entries[index].cwd != cwd;
        let stale_generation = self.entries[index]
            .live
            .as_ref()
            .is_some_and(|live| live.generation != generation);

        if cwd_changed || stale_generation {
            let released = self.release_live(index);
            self.entries[index].cwd = cwd.to_path_buf();
            // A directory change makes the recorded options meaningless only if
            // the agent reports different ones; keeping them lets the replay
            // restore the user's pick and the agent correct it if it cannot.
            return (self.create_decision(index), released);
        }

        match &self.entries[index].live {
            Some(live) => (
                WarmDecision::Ready {
                    session_id: self.entries[index].session_id.clone(),
                    agent_session_id: live.agent_session_id.clone(),
                    config_options: self.entries[index].config_options.clone(),
                },
                None,
            ),
            None => (self.create_decision(index), None),
        }
    }

    /// Records the provider session produced for a `Create` decision.
    ///
    /// A decision whose entry disappeared while the handshake was in flight is
    /// reported back so the caller releases the orphaned provider session.
    pub(super) fn commit_created(
        &mut self,
        session_id: &SessionId,
        created: CreatedProvider,
        generation: u64,
        now: i64,
    ) -> Option<ReleasedSession> {
        let index = self.index_of(session_id)?;
        let released = self.release_live(index);
        let entry = &mut self.entries[index];
        entry.live = Some(LiveSession {
            agent_session_id: created.agent_session_id,
            generation,
        });
        entry.config_options = created.config_options;
        entry.available_commands = created.available_commands;
        entry.last_used_at = now;
        released
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
                agent_cli: entry.key.agent_cli,
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

    /// Removes one warm session from the pool so it can be persisted.
    ///
    /// Returns `None` when the entry is cold or missing; the caller then rebuilds
    /// a provider session before attaching rather than failing the user's prompt.
    pub(super) fn take_for_attach(&mut self, session_id: &SessionId) -> Option<AttachedWarm> {
        let index = self.index_of(session_id)?;
        // A cold entry stays in the pool: it still carries the cwd and the
        // options needed to rebuild, which `rebuild_plan` reads next.
        self.entries[index].live.as_ref()?;
        let entry = self.entries.remove(index);
        let live = entry.live?;
        Some(AttachedWarm {
            session_id: entry.session_id,
            agent_cli: entry.key.agent_cli,
            agent_session_id: live.agent_session_id,
            cwd: entry.cwd,
            available_commands: entry.available_commands,
        })
    }

    /// Removes one warm entry outright and reports any provider session it held.
    ///
    /// Used when a rebuilt session replaces the entry, so the superseded one is
    /// released rather than left running unreferenced.
    pub(super) fn forget(&mut self, session_id: &SessionId) -> Option<ReleasedSession> {
        let index = self.index_of(session_id)?;
        let released = self.release_live(index);
        self.entries.remove(index);
        released
    }

    /// Returns what a cold or missing entry needs in order to be rebuilt.
    pub(super) fn rebuild_plan(&self, session_id: &SessionId) -> Option<RebuildPlan> {
        let index = self.index_of(session_id)?;
        let entry = &self.entries[index];
        Some(RebuildPlan {
            agent_cli: entry.key.agent_cli,
            cwd: entry.cwd.clone(),
            replay: entry.desired_config.clone().into_iter().collect(),
        })
    }

    /// Releases every provider session belonging to a superseded connection generation.
    ///
    /// A CLI that crashed and restarted leaves behind identifiers that no longer
    /// resolve. The entries survive as cold so the identifiers clients hold keep
    /// working; only the dead provider sessions are dropped.
    pub(super) fn invalidate_generation(&mut self, agent_cli: AgentCli, generation: u64) {
        for entry in &mut self.entries {
            if entry.key.agent_cli == agent_cli
                && entry
                    .live
                    .as_ref()
                    .is_some_and(|live| live.generation != generation)
            {
                entry.live = None;
            }
        }
    }

    /// Retires sessions that idled out or exceeded the pool's bounds.
    pub(super) fn evict(&mut self, now: i64) -> Vec<ReleasedSession> {
        let mut released = Vec::new();
        for index in 0..self.entries.len() {
            if self.entries[index].live.is_some()
                && now.saturating_sub(self.entries[index].last_used_at) >= IDLE_TTL_MILLIS
            {
                released.extend(self.release_live(index));
            }
        }

        let mut live: Vec<usize> = (0..self.entries.len())
            .filter(|index| self.entries[*index].live.is_some())
            .collect();
        live.sort_by_key(|index| self.entries[*index].last_used_at);
        let over_capacity = live.len().saturating_sub(MAX_LIVE_ENTRIES);
        for index in live.into_iter().take(over_capacity) {
            released.extend(self.release_live(index));
        }

        if self.entries.len() > MAX_ENTRIES {
            let mut order: Vec<usize> = (0..self.entries.len()).collect();
            order.sort_by_key(|index| self.entries[*index].last_used_at);
            let mut doomed: Vec<usize> = order
                .into_iter()
                .take(self.entries.len() - MAX_ENTRIES)
                .collect();
            doomed.sort_unstable_by(|left, right| right.cmp(left));
            for index in doomed {
                released.extend(self.release_live(index));
                self.entries.remove(index);
            }
        }
        released
    }

    /// Builds the create decision for an entry, carrying its recorded choices.
    fn create_decision(&self, index: usize) -> WarmDecision {
        let entry = &self.entries[index];
        WarmDecision::Create {
            session_id: entry.session_id.clone(),
            cwd: entry.cwd.clone(),
            replay: entry.desired_config.clone().into_iter().collect(),
        }
    }

    /// Detaches an entry's provider session and reports it for release.
    fn release_live(&mut self, index: usize) -> Option<ReleasedSession> {
        let agent_cli = self.entries[index].key.agent_cli;
        self.entries[index].live.take().map(|live| ReleasedSession {
            agent_cli,
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
    pub agent_cli: AgentCli,
    pub cwd: PathBuf,
    pub replay: Vec<(SessionConfigId, SessionConfigOptionValue)>,
}

#[cfg(test)]
mod tests {
    use super::{
        AttachedWarm, ConfigTarget, CreatedProvider, IDLE_TTL_MILLIS, MAX_LIVE_ENTRIES,
        RebuildPlan, ReleasedSession, WarmDecision, WarmKey, WarmPool,
    };
    use ora_contracts::WarmSessionTarget;
    use ora_contracts::acp::session_config_options::{
        SessionConfigOption, SessionConfigOptionValue, SessionConfigSelectOption,
    };
    use ora_domain::{AgentCli, SessionId};
    use pretty_assertions::assert_eq;
    use std::path::{Path, PathBuf};

    const GENERATION: u64 = 1;

    /// Builds a key for one task-scoped chat surface.
    fn key(task_id: &str, client_id: &str) -> WarmKey {
        WarmKey {
            target: WarmSessionTarget::Task {
                task_id: task_id.to_string(),
            },
            agent_cli: AgentCli::OpenCode,
            client_id: client_id.to_string(),
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
        let WarmDecision::Create { session_id, .. } = decision else {
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

    /// Verifies two clients on the same selection never share one provider session.
    #[test]
    fn isolates_warm_sessions_per_client() {
        let mut pool = WarmPool::default();
        let first = warm(
            &mut pool,
            &key("task-1", "client-1"),
            Path::new("/repo"),
            0,
            "session-1",
        );

        let (decision, released) = pool.lookup(
            &key("task-1", "client-2"),
            Path::new("/repo"),
            GENERATION,
            0,
            || SessionId::new("session-2"),
        );

        assert_eq!(
            (decision, released),
            (
                WarmDecision::Create {
                    session_id: SessionId::new("session-2"),
                    cwd: PathBuf::from("/repo"),
                    replay: vec![],
                },
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
                WarmDecision::Create {
                    session_id,
                    cwd: PathBuf::from("/repo/new"),
                    replay: vec![],
                },
                Some(ReleasedSession {
                    agent_cli: AgentCli::OpenCode,
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

        pool.invalidate_generation(AgentCli::OpenCode, GENERATION + 1);
        let (decision, released) = pool.lookup(&key, Path::new("/repo"), GENERATION + 1, 5, || {
            SessionId::new("unused")
        });

        assert_eq!(
            (decision, released),
            (
                WarmDecision::Create {
                    session_id,
                    cwd: PathBuf::from("/repo"),
                    replay: vec![],
                },
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

        pool.invalidate_generation(AgentCli::OpenCode, GENERATION + 1);
        let (decision, _) = pool.lookup(&key, Path::new("/repo"), GENERATION + 1, 5, || {
            SessionId::new("unused")
        });

        assert_eq!(
            decision,
            WarmDecision::Create {
                session_id,
                cwd: PathBuf::from("/repo"),
                replay: vec![("model".into(), SessionConfigOptionValue::value_id("smart"),)],
            }
        );
    }

    /// Verifies a cold session records choices without paying for a provider session.
    #[test]
    fn defers_configuration_for_a_cold_session() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");
        pool.invalidate_generation(AgentCli::OpenCode, GENERATION + 1);

        assert_eq!(
            pool.config_target(&session_id, 5),
            Some(ConfigTarget::Deferred)
        );
    }

    /// Verifies an idle session is released while its entry survives for reuse.
    #[test]
    fn releases_sessions_that_idled_out() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");

        let released = pool.evict(IDLE_TTL_MILLIS);

        assert_eq!(
            (released, pool.config_target(&session_id, IDLE_TTL_MILLIS)),
            (
                vec![ReleasedSession {
                    agent_cli: AgentCli::OpenCode,
                    agent_session_id: "agent-session-1".to_string(),
                    generation: GENERATION,
                }],
                Some(ConfigTarget::Deferred),
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
            pool.evict(MAX_LIVE_ENTRIES as i64),
            vec![ReleasedSession {
                agent_cli: AgentCli::OpenCode,
                agent_session_id: "agent-session-0".to_string(),
                generation: GENERATION,
            }]
        );
    }

    /// Verifies attaching consumes the entry so the next visit warms a fresh session.
    #[test]
    fn consumes_the_entry_when_it_is_attached() {
        let mut pool = WarmPool::default();
        let key = key("task-1", "client-1");
        let session_id = warm(&mut pool, &key, Path::new("/repo"), 0, "session-1");

        let attached = pool.take_for_attach(&session_id);
        let (decision, _) = pool.lookup(&key, Path::new("/repo"), GENERATION, 5, || {
            SessionId::new("session-2")
        });

        assert_eq!(
            (attached, decision),
            (
                Some(AttachedWarm {
                    session_id,
                    agent_cli: AgentCli::OpenCode,
                    agent_session_id: "agent-session-1".to_string(),
                    cwd: PathBuf::from("/repo"),
                    available_commands: vec![],
                }),
                WarmDecision::Create {
                    session_id: SessionId::new("session-2"),
                    cwd: PathBuf::from("/repo"),
                    replay: vec![],
                },
            )
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
        pool.invalidate_generation(AgentCli::OpenCode, GENERATION + 1);

        assert_eq!(
            (
                pool.take_for_attach(&session_id),
                pool.rebuild_plan(&session_id)
            ),
            (
                None,
                Some(RebuildPlan {
                    agent_cli: AgentCli::OpenCode,
                    cwd: PathBuf::from("/repo"),
                    replay: vec![("model".into(), SessionConfigOptionValue::value_id("smart"),)],
                }),
            )
        );
    }
}
