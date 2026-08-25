use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

/// Per-key shared/exclusive coordination for physical resource use.
///
/// One instance keys worktree-use leases by Workspace id (consumers acquire shared,
/// cleanup acquires exclusive) and another keys repository mutation gates by
/// normalized repository root (exclusive only). Entries are removed as soon as
/// no holder or waiter references the key, so the map does not grow with
/// historical tasks.
///
/// Writers are preferred: new shared acquisitions wait while a writer waits,
/// so a cleanup cannot be starved by a stream of readers. Guards own their key
/// and release on drop. All waiting is process-internal and short-lived by
/// design — callers hold guards only around blocking filesystem/Git/SQLite
/// work, never across an async await point.
#[derive(Debug, Default)]
pub struct KeyedResourceLocks {
    states: Mutex<HashMap<String, LockState>>,
    changed: Condvar,
}

/// Tracks the holders and waiters of one key.
#[derive(Debug, Default, Clone, Copy)]
struct LockState {
    readers: usize,
    writer: bool,
    waiting_writers: usize,
}

impl LockState {
    /// Reports whether the entry can be dropped from the map entirely.
    fn is_idle(&self) -> bool {
        self.readers == 0 && !self.writer && self.waiting_writers == 0
    }
}

impl KeyedResourceLocks {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Locks the state map, recovering from a panicked holder's poison.
    ///
    /// The state is a plain counter table with no invariant that a panicking
    /// holder could have half-applied, so continuing with the inner value is
    /// always safe and keeps one panicked consumer from wedging all cleanup.
    fn lock_states(&self) -> MutexGuard<'_, HashMap<String, LockState>> {
        self.states.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Acquires the key for shared use, waiting out any active or waiting writer.
    pub fn acquire_shared(self: &Arc<Self>, key: impl Into<String>) -> SharedLeaseGuard {
        let key = key.into();
        let mut states = self.lock_states();
        loop {
            let state = states.entry(key.clone()).or_default();
            if !state.writer && state.waiting_writers == 0 {
                state.readers += 1;
                return SharedLeaseGuard {
                    locks: Arc::clone(self),
                    key,
                };
            }
            states = self
                .changed
                .wait(states)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Acquires the key exclusively, waiting for every current holder to finish.
    pub fn acquire_exclusive(self: &Arc<Self>, key: impl Into<String>) -> ExclusiveLeaseGuard {
        let key = key.into();
        let mut states = self.lock_states();
        states.entry(key.clone()).or_default().waiting_writers += 1;
        loop {
            // The waiting-writer count registered above keeps this entry alive
            // across waits, so or_default only ever re-reads the same entry.
            let state = states.entry(key.clone()).or_default();
            if state.readers == 0 && !state.writer {
                state.waiting_writers -= 1;
                state.writer = true;
                return ExclusiveLeaseGuard {
                    locks: Arc::clone(self),
                    key,
                };
            }
            states = self
                .changed
                .wait(states)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Releases one holder and garbage-collects the entry when idle.
    fn release(&self, key: &str, exclusive: bool) {
        let mut states = self.lock_states();
        if let Some(state) = states.get_mut(key) {
            if exclusive {
                state.writer = false;
            } else {
                state.readers -= 1;
            }
            if state.is_idle() {
                states.remove(key);
            }
        }
        self.changed.notify_all();
    }
}

/// Holds one shared lease until dropped.
#[derive(Debug)]
pub struct SharedLeaseGuard {
    locks: Arc<KeyedResourceLocks>,
    key: String,
}

impl Drop for SharedLeaseGuard {
    fn drop(&mut self) {
        self.locks.release(&self.key, /*exclusive*/ false);
    }
}

/// Holds one exclusive lease until dropped.
#[derive(Debug)]
pub struct ExclusiveLeaseGuard {
    locks: Arc<KeyedResourceLocks>,
    key: String,
}

impl Drop for ExclusiveLeaseGuard {
    fn drop(&mut self) {
        self.locks.release(&self.key, /*exclusive*/ true);
    }
}

#[cfg(test)]
mod tests {
    use super::KeyedResourceLocks;
    use pretty_assertions::assert_eq;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;

    /// Verifies exclusive acquisition waits for in-flight shared holders.
    #[test]
    fn exclusive_waits_for_shared_holders() {
        let locks = KeyedResourceLocks::new();
        let shared = locks.acquire_shared("task-1");
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let worker_locks = locks.clone();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).expect("report waiting");
            let _exclusive = worker_locks.acquire_exclusive("task-1");
            acquired_tx.send(()).expect("report acquired");
        });
        started_rx.recv().expect("worker started");
        assert!(
            acquired_rx.try_recv().is_err(),
            "exclusive must not be granted while a shared holder is live"
        );
        drop(shared);
        acquired_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("exclusive granted after shared release");
        worker.join().expect("worker finished");
    }

    /// Verifies unrelated keys do not contend and idle entries are collected.
    #[test]
    fn keys_are_independent_and_collected() {
        let locks = KeyedResourceLocks::new();
        {
            let _a = locks.acquire_exclusive("repo-a");
            let _b = locks.acquire_shared("repo-b");
        }
        let states = locks.states.lock().expect("state lock");
        assert_eq!(states.len(), 0);
    }

    /// Verifies concurrent shared holders proceed together for one key.
    #[test]
    fn shared_holders_do_not_block_each_other() {
        let locks = KeyedResourceLocks::new();
        static CONCURRENT: AtomicUsize = AtomicUsize::new(0);
        let first = locks.acquire_shared("task-1");
        let second = locks.acquire_shared("task-1");
        CONCURRENT.fetch_add(2, Ordering::SeqCst);
        assert_eq!(CONCURRENT.load(Ordering::SeqCst), 2);
        drop(first);
        drop(second);
    }
}
