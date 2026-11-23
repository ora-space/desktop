//! Fair, value-free probe admission. A task owns its permit through cleanup, including cancellation.
use super::control::Control;
use ora_node_protocol::{ExecutionId, SessionMcpUpdateMember};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, PoisonError},
};
mod runner;
pub(crate) use runner::register;
#[cfg(test)]
mod tests;

/// Values are captured only after a global permit has been assigned.
pub(crate) struct ProbeInput {
    pub generation: u64,
    pub attempt: u64,
    pub member: SessionMcpUpdateMember,
}
type Runner = dyn Fn(Arc<ProbeInput>) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync;
struct Entry {
    control: Control,
    runner: Arc<Runner>,
}
#[derive(Default)]
struct State {
    entries: HashMap<ExecutionId, Entry>,
    order: VecDeque<ExecutionId>,
    active: HashMap<ExecutionId, usize>,
    retained: usize,
    owners: HashMap<ExecutionId, Control>,
}

/// One instance per Node, shared by every session; no session can consume more than two slots.
#[derive(Clone, Default)]
pub(crate) struct Scheduler(Arc<Mutex<State>>, pub(crate) Arc<tokio::sync::Notify>);

impl Scheduler {
    /// Root diagnostics include anonymous global counts, including cleanup held after unregister.
    pub(crate) fn summary(&self) -> crate::session::stop::admin::Reply {
        let state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let active = state.active.values().sum::<usize>();
        let cleanup = state.retained
            + state
                .owners
                .values()
                .map(|control| control.probe_counts().1)
                .sum::<usize>();
        let queued = state
            .entries
            .values()
            .map(|entry| entry.control.probe_counts().0)
            .sum();
        crate::session::stop::admin::Reply::ProbeCapacity {
            total: 8,
            running: active.saturating_sub(cleanup.saturating_sub(state.retained)),
            cleanup_pending: cleanup,
            queued,
            available: 8usize.saturating_sub(active + state.retained),
            residues: vec![],
        }
    }
    /// Restored cleanup responsibilities consume capacity before accepting fresh work.
    pub(crate) fn retain_recovered(&self, count: usize) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retained = count;
    }

    /// Publishes only anonymous global capacity, never another session's identity.
    pub(crate) fn available(&self) -> bool {
        let state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.retained + state.active.values().sum::<usize>() < 8
    }

    /// Registers a session context without retaining any resolved settings in the queue.
    pub(crate) fn register(
        &self,
        execution: ExecutionId,
        control: Control,
        runner: impl Fn(Arc<ProbeInput>) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + Sync
        + 'static,
    ) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if !state.entries.contains_key(&execution) {
            state.order.push_back(execution.clone());
        }
        state.entries.insert(
            execution,
            Entry {
                control,
                runner: Arc::new(runner),
            },
        );
    }

    /// Removes queued intent; already spawned tasks keep their owner and slot until they finish.
    pub(crate) fn unregister(&self, execution: &ExecutionId) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        state.entries.remove(execution);
        state.order.retain(|id| id != execution);
    }

    /// Visits each session once per round and moves a winner to the tail before spawning work.
    pub(crate) fn dispatch(&self) {
        loop {
            let next = {
                let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
                if state.retained + state.active.values().sum::<usize>() >= 8 {
                    return;
                }
                let mut selected = None;
                for _ in 0..state.order.len() {
                    let Some(execution) = state.order.pop_front() else {
                        break;
                    };
                    state.order.push_back(execution.clone());
                    if state.active.get(&execution).copied().unwrap_or(0) >= 2 {
                        continue;
                    }
                    let Some(entry) = state.entries.get(&execution) else {
                        continue;
                    };
                    let Some(input) = entry.control.claim_probe() else {
                        continue;
                    };
                    let runner = Arc::clone(&entry.runner);
                    let control = entry.control.clone();
                    state.owners.insert(execution.clone(), control);
                    *state.active.entry(execution.clone()).or_default() += 1;
                    selected = Some((execution, runner, input));
                    break;
                }
                selected
            };
            let Some((execution, runner, input)) = next else {
                return;
            };
            let owner = self.clone();
            tokio::spawn(async move {
                runner(input).await;
                {
                    let mut state = owner.0.lock().unwrap_or_else(PoisonError::into_inner);
                    if let Some(active) = state.active.get_mut(&execution) {
                        *active -= 1;
                        if *active == 0 {
                            state.owners.remove(&execution);
                        }
                    }
                }
                owner.dispatch();
            });
        }
    }
}
