//! One spawn gate and inherited OFD for the plugin runtime and every host-managed subprocess.
use super::files::StopFiles;
use ora_node_db::SessionStopIdentity;
use ora_process::{
    ManagedProcess, ProcessSpawner, ProcessSpec, TokioManagedProcess, TokioProcessSpawner,
};
use std::{
    fmt, io,
    os::fd::OwnedFd,
    sync::{Arc, Mutex, PoisonError},
};

#[derive(Clone)]
pub(crate) struct SessionAttemptSpawner {
    inner: TokioProcessSpawner,
    attempt: Arc<Attempt>,
}
struct Attempt {
    identity: SessionStopIdentity,
    files: StopFiles,
    descriptor: Mutex<Option<Arc<OwnedFd>>>,
}

impl fmt::Debug for SessionAttemptSpawner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionAttemptSpawner")
            .field("attempt", &self.attempt.identity.attempt)
            .finish()
    }
}

impl SessionAttemptSpawner {
    /// Construction follows durable attempt registration; every clone shares this same gate.
    pub(crate) fn new(
        inner: TokioProcessSpawner,
        identity: SessionStopIdentity,
        files: StopFiles,
    ) -> io::Result<Self> {
        let descriptor = files.hold(&identity.attempt)?;
        Ok(Self {
            inner,
            attempt: Arc::new(Attempt {
                identity,
                files,
                descriptor: Mutex::new(Some(descriptor)),
            }),
        })
    }
    /// Serializes with spawn, refuses future children, then drops Node's last retained OFD copy.
    pub(crate) fn close(&self) {
        self.attempt
            .descriptor
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
    }
}

impl ProcessSpawner for SessionAttemptSpawner {
    type Process = TokioManagedProcess;
    fn spawn(&self, spec: ProcessSpec) -> io::Result<Self::Process> {
        let guard = self
            .attempt
            .descriptor
            .lock()
            .map_err(|_| io::Error::other("session spawn gate poisoned"))?;
        let descriptor = guard
            .as_ref()
            .ok_or_else(|| io::Error::other("session attempt is stopping"))?;
        let process = self
            .inner
            .spawn(spec.inherit_descriptor(descriptor.clone()))?;
        if process.id().is_none_or(|pid| {
            self.attempt
                .files
                .note_child(
                    &self.attempt.identity.attempt,
                    &self.attempt.identity.host,
                    pid,
                )
                .is_err()
        }) {
            // The inherited OFD covers a crash before the clue; an observed write failure is
            // explicitly retained rather than using absence of a clue as affirmative evidence.
            if self
                .attempt
                .files
                .mark_incomplete(&self.attempt.identity.attempt)
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
