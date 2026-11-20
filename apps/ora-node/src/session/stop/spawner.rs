//! One spawn gate, inherited OFD and attempt mark for the plugin runtime and every host-managed
//! subprocess.
use super::{files::StopFiles, marks};
use ora_node_db::SessionStopIdentity;
use ora_process::{
    ManagedProcess, ProcessSpawner, ProcessSpec, TokioManagedProcess, TokioProcessSpawner,
};
use std::{
    ffi::OsString,
    fmt, io,
    os::fd::OwnedFd,
    sync::{Arc, Mutex, PoisonError},
};

#[derive(Clone)]
pub(crate) struct SessionAttemptSpawner {
    inner: TokioProcessSpawner,
    attempt: Arc<Mutex<Arc<Attempt>>>,
}
struct Attempt {
    identity: SessionStopIdentity,
    files: StopFiles,
    descriptor: Mutex<Option<Arc<OwnedFd>>>,
    /// The mark entry every child receives, whatever environment its spawner passes on.
    mark: (String, OsString),
}

impl fmt::Debug for SessionAttemptSpawner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionAttemptSpawner")
            .field(
                "attempt",
                &self
                    .attempt
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .identity
                    .attempt,
            )
            .finish()
    }
}

impl SessionAttemptSpawner {
    /// Rotates the shared gate only after the caller has proven the previous attempt stopped.
    pub(crate) fn replace(&self, next: Self) {
        let next = next
            .attempt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        *self.attempt.lock().unwrap_or_else(PoisonError::into_inner) = next;
    }
    /// The original identity survives closing its gate and is used for affirmative stop proof.
    pub(crate) fn identity(&self) -> SessionStopIdentity {
        self.attempt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .identity
            .clone()
    }
    /// Construction follows durable attempt registration; every clone shares this same gate.
    pub(crate) fn new(
        inner: TokioProcessSpawner,
        identity: SessionStopIdentity,
        files: StopFiles,
    ) -> io::Result<Self> {
        let descriptor = files.hold(&identity.attempt)?;
        let mark = marks::mark(&identity.attempt)?;
        // A Node that itself runs inside another launch's mark passes that mark on as well, so
        // the enclosing launch still finds this attempt's processes when it stops.
        let value = mark.child_value(std::env::var_os(mark.name()).as_deref());
        Ok(Self {
            inner,
            attempt: Arc::new(Mutex::new(Arc::new(Attempt {
                identity,
                files,
                descriptor: Mutex::new(Some(descriptor)),
                mark: (mark.name().to_owned(), value),
            }))),
        })
    }
    /// Connection loss can trigger a restart only while the execution admits children.
    pub(crate) fn is_open(&self) -> bool {
        self.attempt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .descriptor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    /// Signal the original groups before waiting for lifecycle cleanup during replacement.
    pub(in crate::session) async fn stop_for_replacement<T>(
        &self,
        host: &ora_node_db::SessionStopHost,
        cleanup: impl std::future::Future<Output = T>,
    ) -> T {
        // Close the gate under the lock, but scan and signal outside it and off the runtime:
        // termination walks procfs, which must hold neither the spawn gate nor the Node.
        let attempt = {
            let attempt = self.attempt.lock().unwrap_or_else(PoisonError::into_inner);
            attempt
                .descriptor
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            Arc::clone(&attempt)
        };
        attempt
            .files
            .terminate_off_runtime(&attempt.identity, host)
            .await;
        cleanup.await
    }

    /// Serializes with spawn, refuses future children, then drops Node's last retained OFD copy.
    pub(crate) fn close(&self) {
        self.attempt
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .descriptor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

impl ProcessSpawner for SessionAttemptSpawner {
    type Process = TokioManagedProcess;
    fn spawn(&self, spec: ProcessSpec) -> io::Result<Self::Process> {
        let attempt = self.attempt.lock().unwrap_or_else(PoisonError::into_inner);
        let guard = attempt
            .descriptor
            .lock()
            .map_err(|_| io::Error::other("session spawn gate poisoned"))?;
        let descriptor = guard
            .as_ref()
            .ok_or_else(|| io::Error::other("session attempt is stopping"))?;
        // Set last, so neither a cleared environment nor a caller's own entry can drop the mark.
        let (name, value) = &attempt.mark;
        let process = self
            .inner
            .spawn(spec.env(name, value).inherit_descriptor(descriptor.clone()))?;
        if process.id().is_none_or(|pid| {
            attempt
                .files
                .note_child(&attempt.identity.attempt, &attempt.identity.host, pid)
                .is_err()
        }) {
            // The inherited OFD covers a crash before the clue; an observed write failure is
            // explicitly retained rather than using absence of a clue as affirmative evidence.
            if attempt
                .files
                .mark_incomplete(&attempt.identity.attempt)
                .is_err()
            {
                return Err(io::Error::other(
                    "session process clue could not be retained",
                ));
            }
        }
        Ok(process)
    }
}
