//! Batches deferred consumer activations so each shared Agent is activated once per claim batch.
//!
//! ADR-0015 bounds the first MCP→Agent loop: a worker reuses each Agent's existing wait/restart
//! coordination but groups its current claim batch so each shared Agent is activated at most once.
//! The per-surface reconciles quiesce and mutate their files, but the restart that makes the shared
//! process consume the new config is deferred — collected here and flushed once, after every
//! surface a consumer serves has been written. Activating between two surfaces that share one agent
//! would restart it before the later write and miss it; flushing after the whole batch is what
//! makes the "at most once" bound hold without a durable global exactly-once cohort.

use ora_effect::{
    Condition, ConditionReason, ConditionSubject, ConsumerId, ConsumerStatus, CoordinationError,
    Generation, SurfaceKey, SurfacePath, SurfacePhase,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::PoisonError;

/// One surface's deferred activation request, captured when its per-surface resume would otherwise
/// restart the shared Agent immediately.
///
/// Carries the locator the production activator needs to address one restart IPC, plus the
/// generation the surface reached, so a failed activation can stamp the condition that explains why
/// the surface is not yet ready for the generation it already wrote to disk.
#[derive(Clone, Debug)]
pub(crate) struct ActivationSurface {
    pub(crate) surface_key: SurfaceKey,
    pub(crate) workspace_root: PathBuf,
    pub(crate) relative_path: SurfacePath,
    pub(crate) generation: Generation,
}

/// The per-batch ledger of consumer activations deferred from per-surface resumes.
///
/// Interior mutability lets a `&BatchActivation` travel through every per-surface coordinator in one
/// claim batch — each of which is a short-lived borrow — while a single flush reads and drains it
/// after the batch.
pub(crate) struct BatchActivation {
    pending: Mutex<BTreeMap<ConsumerId, ConsumerActivation>>,
}

#[derive(Debug, Default)]
struct ConsumerActivation {
    /// Every surface this consumer was asked to resume for in the batch, in arrival order.
    surfaces: Vec<ActivationSurface>,
    /// Whether any surface in the batch held this consumer's mutation barrier (quiesced), which is
    /// what tells the flush the restart will replace the Agent's process and detach its sessions.
    barriered: bool,
}

impl BatchActivation {
    pub(crate) fn new() -> Self {
        Self {
            pending: Mutex::new(BTreeMap::new()),
        }
    }

    /// Records one surface's deferred resume so the flush can activate its consumer once.
    ///
    /// `barriered` is the per-surface coordinator's quiesce result: only a surface that actually
    /// held the Agent's mutation barrier is about to have its process replaced, so the flush detaches
    /// sessions only when at least one recorded surface for that consumer was barriered.
    pub(crate) fn record(
        &self,
        consumer: &ConsumerId,
        surface: ActivationSurface,
        barriered: bool,
    ) {
        let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
        let activation = pending.entry(consumer.clone()).or_default();
        activation.barriered |= barriered;
        activation.surfaces.push(surface);
    }
}

/// Activates each deferred consumer, returning the consumer statuses that must be persisted as
/// Degraded because their activation failed (an empty vec when every activation succeeded).
///
/// The per-surface reconcile already persisted a Current consumer status for every resume that
/// returned Ok; on success that stands and the flush persists nothing, and on failure it overwrites
/// the per-surface status with Degraded so the surface is not reported ready for a process that did
/// not consume the applied config.
pub(crate) fn flush_batch_activation(
    activation: &BatchActivation,
    now: i64,
    activate: impl Fn(&ConsumerId, &ActivationSurface, bool) -> Result<(), CoordinationError>,
) -> Vec<ConsumerStatus> {
    let pending = std::mem::take(
        &mut *activation
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    let mut degraded = Vec::new();
    for (consumer, act) in &pending {
        // One activation per consumer is the whole point of the batch: the shared Agent process is
        // restarted once, after every surface it consumes has been written. Activating per surface
        // would restart it between two of its own surfaces and miss the later write.
        let Some(first) = act.surfaces.first() else {
            continue;
        };
        if activate(consumer, first, act.barriered).is_ok() {
            continue;
        }
        // The agent did not consume the applied config, so every surface it served for this batch is
        // marked Degraded at its own written generation until a later activation converges it.
        for surface in &act.surfaces {
            degraded.push(ConsumerStatus {
                surface_key: surface.surface_key.clone(),
                consumer_id: consumer.clone(),
                ready_generation: Generation::default(),
                phase: SurfacePhase::Degraded,
                revision: 1,
                updated_at: now,
                conditions: vec![Condition::new(
                    ConditionSubject::Consumer {
                        consumer_id: consumer.clone(),
                    },
                    ConditionReason::ConsumerResumeFailed,
                    "batched agent activation failed",
                    now,
                    surface.generation,
                )],
            });
        }
    }
    degraded
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;
    use std::sync::{Arc, Mutex};

    fn surface(key: &str, path: &str) -> ActivationSurface {
        ActivationSurface {
            surface_key: SurfaceKey::new(key),
            workspace_root: PathBuf::from("/workspace"),
            relative_path: SurfacePath::parse(path).expect("surface path"),
            generation: Generation::new(1),
        }
    }

    /// Two surfaces sharing one consumer activate it at most once: the batch defers every resume and
    /// flushes one activation per consumer after every surface it consumes has been written, so a
    /// shared Agent is never restarted between its own surfaces (which would restart it before the
    /// later write and miss it).
    #[test]
    fn two_surfaces_sharing_one_consumer_activate_it_at_most_once() {
        let activation = BatchActivation::new();
        let consumer = ConsumerId::new("official/ora-space.opencode");
        activation.record(&consumer, surface("skills", ".opencode/skills"), true);
        activation.record(&consumer, surface("mcp", ".opencode/opencode.jsonc"), true);

        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorder = calls.clone();
        let degraded =
            flush_batch_activation(&activation, 0, move |consumer, _surface, _barriered| {
                recorder
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(consumer.as_str().to_string());
                Ok(())
            });
        assert_eq!(
            calls.lock().unwrap_or_else(PoisonError::into_inner).len(),
            1,
            "two surfaces sharing one consumer must activate it once, not once per surface",
        );
        assert!(
            degraded.is_empty(),
            "a successful activation reports nothing to persist as Degraded",
        );
    }

    /// Distinct consumers each activate once, so batching never starves an Agent that shares a batch
    /// with another.
    #[test]
    fn distinct_consumers_each_activate_once() {
        let activation = BatchActivation::new();
        activation.record(
            &ConsumerId::new("official/a"),
            surface("s1", ".a/skills"),
            true,
        );
        activation.record(
            &ConsumerId::new("official/b"),
            surface("s2", ".b/opencode.jsonc"),
            true,
        );

        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorder = calls.clone();
        flush_batch_activation(&activation, 0, move |consumer, _surface, _barriered| {
            recorder
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(consumer.as_str().to_string());
            Ok(())
        });
        assert_eq!(
            calls.lock().unwrap_or_else(PoisonError::into_inner).len(),
            2,
            "two distinct consumers must each activate once",
        );
    }

    /// A failed activation reports one Degraded status per surface the consumer serves, so every
    /// surface is marked not-ready for the generation it wrote until the Agent consumes it.
    #[test]
    fn a_failed_activation_marks_every_served_surface_degraded() {
        let activation = BatchActivation::new();
        let consumer = ConsumerId::new("official/ora-space.opencode");
        activation.record(&consumer, surface("skills", ".opencode/skills"), true);
        activation.record(&consumer, surface("mcp", ".opencode/opencode.jsonc"), false);

        let degraded =
            flush_batch_activation(&activation, 100, |_consumer, _surface, _barriered| {
                Err(CoordinationError::new(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "activation failed",
                )))
            });
        assert_eq!(
            degraded.len(),
            2,
            "both surfaces the failed consumer serves become Degraded",
        );
        assert!(
            degraded
                .iter()
                .all(|status| status.phase == SurfacePhase::Degraded)
        );
        assert_eq!(
            degraded
                .iter()
                .map(|status| status.surface_key.clone())
                .collect::<Vec<_>>(),
            vec![SurfaceKey::new("skills"), SurfaceKey::new("mcp")],
            "Degraded statuses preserve the arrival order of the surfaces the consumer served",
        );
    }
}
