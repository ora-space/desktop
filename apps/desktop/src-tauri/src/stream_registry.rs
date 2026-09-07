//! Owns stream registrations from startup through forwarding and application shutdown.

use ora_backend::{BackendError, ErrorClassification};
use ora_contracts::{EmptyErrorParams, PublicError};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// Shares cancellation with the command, its startup work, and the eventual forwarding task.
#[derive(Clone, Default)]
pub(crate) struct StreamRegistry {
    inner: Arc<RegistryState>,
}

#[derive(Default)]
struct RegistryState {
    registrations: Mutex<HashMap<String, CancellationToken>>,
    shutdown: CancellationToken,
}

/// Holds exclusive ownership of a stream id until startup or forwarding releases it.
pub(crate) struct StreamRegistration {
    registry: StreamRegistry,
    id: String,
    cancellation: CancellationToken,
}

impl StreamRegistry {
    /// Claims an id before startup so duplicate or cancelled calls cannot create untracked work.
    pub(crate) fn register(&self, id: String) -> Result<StreamRegistration, BackendError> {
        let mut registrations = self.inner.registrations.lock().map_err(|_poisoned| {
            BackendError::internal(
                "stream registry is unavailable",
                std::io::Error::other("registry lock poisoned"),
            )
        })?;
        if self.inner.shutdown.is_cancelled() {
            return Err(BackendError::new(
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "Desktop streams are shutting down",
            ));
        }
        if registrations.contains_key(&id) {
            return Err(BackendError::new(
                ErrorClassification::Conflict,
                PublicError::InvalidRequest(EmptyErrorParams {}),
                "stream call id is already registered",
            ));
        }
        let cancellation = self.inner.shutdown.child_token();
        registrations.insert(id.clone(), cancellation.clone());
        Ok(StreamRegistration {
            registry: self.clone(),
            id,
            cancellation,
        })
    }

    /// Signals cancellation without releasing the id while older startup or forwarding still owns it.
    pub(crate) fn cancel(&self, id: &str) -> Result<(), BackendError> {
        let registrations = self.inner.registrations.lock().map_err(|_poisoned| {
            BackendError::internal(
                "stream registry is unavailable",
                std::io::Error::other("registry lock poisoned"),
            )
        })?;
        if let Some(cancellation) = registrations.get(id) {
            cancellation.cancel();
        }
        Ok(())
    }

    /// Cancels both starting and running streams and rejects every later registration.
    pub(crate) fn shutdown(&self) {
        self.inner.shutdown.cancel();
    }
}

impl StreamRegistration {
    /// Lets startup and forwarding observe the same cancellation, including application shutdown.
    pub(crate) fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }
}

impl Drop for StreamRegistration {
    /// Releases only this owner's id; cancellation deliberately cannot make it reusable earlier.
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Ok(mut registrations) = self.registry.inner.registrations.lock() {
            registrations.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StreamRegistry;
    use ora_logging::with_trace_logging;

    /// A cancelled creator retains its id until its resources have actually been released.
    #[test]
    fn cancellation_during_startup_cannot_release_another_generation() {
        with_trace_logging(|| {
            let registry = StreamRegistry::default();
            let registration = registry
                .register("fixture".to_string())
                .expect("register stream");
            assert!(registry.register("fixture".to_string()).is_err());
            registry.cancel("fixture").expect("cancel during startup");
            assert!(registration.cancellation().is_cancelled());
            assert!(registry.register("fixture".to_string()).is_err());
            drop(registration);
            let replacement = registry
                .register("fixture".to_string())
                .expect("reuse released id");
            assert!(!replacement.cancellation().is_cancelled());
        });
    }

    /// Shutdown reaches every phase and a cancelled runtime cannot accept fresh subscriptions.
    #[test]
    fn shutdown_cancels_all_registrations_and_prevents_reopening() {
        with_trace_logging(|| {
            let registry = StreamRegistry::default();
            let starting = registry
                .register("starting".to_string())
                .expect("starting stream");
            let running = registry
                .register("running".to_string())
                .expect("running stream");
            registry.shutdown();
            assert!(starting.cancellation().is_cancelled());
            assert!(running.cancellation().is_cancelled());
            assert!(registry.register("later".to_string()).is_err());
            registry
                .cancel("already-finished")
                .expect("late cancellation is idempotent");
        });
    }
}
