use crate::{BackendError, ErrorClassification};
use ora_contracts::RequestId;
use ora_logging::{ErrorReport, ora_debug, ora_error, ora_info, ora_warn};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

/// Generates canonical Ora request identifiers at runtime adapter entry seams.
pub trait RequestIdGenerator: Send + Sync {
    fn generate(&self) -> RequestId;
}

/// Generates production request identifiers using UUID version four.
#[derive(Clone, Copy, Debug, Default)]
pub struct UuidRequestIdGenerator;

impl RequestIdGenerator for UuidRequestIdGenerator {
    fn generate(&self) -> RequestId {
        RequestId::new_v4()
    }
}

struct RequestLifecycleInner {
    request_id: RequestId,
    operation: Arc<str>,
    started_at: Instant,
    completed: AtomicBool,
}

impl RequestLifecycleInner {
    fn duration_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }
}

impl Drop for RequestLifecycleInner {
    /// Records a completion for requests that lose their last handle without claiming an outcome.
    ///
    /// This makes "exactly one completion event per request" a structural guarantee instead of a
    /// per-call-site checklist: a future seam that forgets `complete_*`, or a deferred stream whose
    /// future is dropped when its transport disappears, still closes its record. The outcome is
    /// distinct so those cases stay greppable, and the level matches cancellation because the
    /// common cause is caller teardown rather than a backend failure.
    fn drop(&mut self) {
        if self.completed.load(Ordering::Acquire) {
            return;
        }

        ora_debug!(
            operation = self.operation.as_ref(),
            request_id = %self.request_id,
            outcome = "abandoned",
            duration_ms = self.duration_ms(),
            "request completed"
        );
    }
}

/// Coordinates correlation and exactly-once completion logging across cloned async handles.
#[derive(Clone)]
pub struct RequestLifecycle {
    inner: Arc<RequestLifecycleInner>,
}

impl RequestLifecycle {
    /// Starts one adapter-owned request using an injected identifier generator.
    pub fn start(operation: impl Into<Arc<str>>, generator: &dyn RequestIdGenerator) -> Self {
        Self {
            inner: Arc::new(RequestLifecycleInner {
                request_id: generator.generate(),
                operation: operation.into(),
                started_at: Instant::now(),
                completed: AtomicBool::new(false),
            }),
        }
    }

    /// Returns the identifier shared by spans, responses, frames, and completion events.
    pub fn request_id(&self) -> RequestId {
        self.inner.request_id
    }

    /// Records a successful request exactly once.
    pub fn complete_success(&self) {
        if !self.claim_completion() {
            return;
        }

        ora_info!(
            operation = self.inner.operation.as_ref(),
            request_id = %self.inner.request_id,
            outcome = "success",
            duration_ms = self.inner.duration_ms(),
            "request completed"
        );
    }

    /// Records a low-noise successful health/readiness request exactly once.
    pub fn complete_success_debug(&self) {
        if !self.claim_completion() {
            return;
        }

        ora_debug!(
            operation = self.inner.operation.as_ref(),
            request_id = %self.inner.request_id,
            outcome = "success",
            duration_ms = self.inner.duration_ms(),
            "request completed"
        );
    }

    /// Records a failed request exactly once using its public classification and sanitized chain.
    pub fn complete_failure(&self, error: &BackendError) {
        if !self.claim_completion() {
            return;
        }

        let report = ErrorReport::from_error(error);
        let code = error.public_error().code();
        match error.classification() {
            ErrorClassification::Internal => ora_error!(
                operation = self.inner.operation.as_ref(),
                request_id = %self.inner.request_id,
                outcome = "failure",
                duration_ms = self.inner.duration_ms(),
                error.code = code,
                error.message = report.message(),
                error.chain = report.chain(),
                error.chain_depth = report.chain_depth(),
                "request completed"
            ),
            ErrorClassification::Conflict => ora_warn!(
                operation = self.inner.operation.as_ref(),
                request_id = %self.inner.request_id,
                outcome = "failure",
                duration_ms = self.inner.duration_ms(),
                error.code = code,
                error.message = report.message(),
                error.chain = report.chain(),
                error.chain_depth = report.chain_depth(),
                "request completed"
            ),
            ErrorClassification::InvalidRequest
            | ErrorClassification::NotFound
            | ErrorClassification::PayloadTooLarge
            | ErrorClassification::Unprocessable => ora_info!(
                operation = self.inner.operation.as_ref(),
                request_id = %self.inner.request_id,
                outcome = "failure",
                duration_ms = self.inner.duration_ms(),
                error.code = code,
                error.message = report.message(),
                error.chain = report.chain(),
                error.chain_depth = report.chain_depth(),
                "request completed"
            ),
        }
    }

    /// Records caller cancellation at debug level without misclassifying it as an internal error.
    pub fn complete_cancellation(&self) {
        if !self.claim_completion() {
            return;
        }

        ora_debug!(
            operation = self.inner.operation.as_ref(),
            request_id = %self.inner.request_id,
            outcome = "cancelled",
            duration_ms = self.inner.duration_ms(),
            "request completed"
        );
    }

    fn claim_completion(&self) -> bool {
        self.inner
            .completed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::{RequestIdGenerator, RequestLifecycle};
    use crate::{BackendError, ErrorClassification};
    use ora_contracts::{EmptyErrorParams, PublicError, RequestId};
    use ora_logging::with_recorded_trace_logging;
    use pretty_assertions::assert_eq;
    use std::sync::{Arc, Mutex};
    use tracing::Level;
    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, Layer};

    struct FixedRequestIdGenerator(RequestId);

    impl RequestIdGenerator for FixedRequestIdGenerator {
        fn generate(&self) -> RequestId {
            self.0
        }
    }

    #[test]
    fn cloned_lifecycles_share_the_id_and_one_completion_claim() {
        let lifecycle = RequestLifecycle::start(
            "test_operation",
            &FixedRequestIdGenerator(test_request_id()),
        );
        let cloned = lifecycle.clone();

        assert_eq!(lifecycle.request_id(), test_request_id());
        assert_eq!(cloned.request_id(), test_request_id());
        assert!(lifecycle.claim_completion());
        assert!(!cloned.claim_completion());
    }

    /// Verifies every backend classification emits its documented completion level.
    #[test]
    fn failure_classifications_map_to_expected_log_levels() {
        let cases = classification_cases();
        let recorder = CompletionRecorder::default();
        with_recorded_trace_logging(recorder.layer(), || {
            for (classification, public_error) in &cases {
                let lifecycle = RequestLifecycle::start(
                    "test_operation",
                    &FixedRequestIdGenerator(test_request_id()),
                );
                lifecycle.complete_failure(&BackendError::new(
                    *classification,
                    public_error.clone(),
                    "test failure",
                ));
            }
        });

        assert_eq!(
            recorder.completions(),
            cases
                .iter()
                .map(|(classification, _)| Completion {
                    level: expected_level(*classification),
                    outcome: Some("failure".to_string()),
                })
                .collect::<Vec<_>>()
        );
    }

    /// A request whose last handle is dropped without an explicit outcome still closes its record.
    #[test]
    fn dropping_an_uncompleted_lifecycle_records_an_abandoned_completion() {
        let recorder = CompletionRecorder::default();
        with_recorded_trace_logging(recorder.layer(), || {
            let lifecycle = RequestLifecycle::start(
                "test_operation",
                &FixedRequestIdGenerator(test_request_id()),
            );
            drop(lifecycle.clone());
            drop(lifecycle);
        });

        assert_eq!(
            recorder.completions(),
            vec![Completion {
                level: Level::DEBUG,
                outcome: Some("abandoned".to_string()),
            }]
        );
    }

    /// The drop fallback never turns an already-recorded completion into a second event.
    #[test]
    fn dropping_a_completed_lifecycle_records_nothing_further() {
        let recorder = CompletionRecorder::default();
        with_recorded_trace_logging(recorder.layer(), || {
            let lifecycle = RequestLifecycle::start(
                "test_operation",
                &FixedRequestIdGenerator(test_request_id()),
            );
            lifecycle.complete_success();
        });

        assert_eq!(
            recorder.completions(),
            vec![Completion {
                level: Level::INFO,
                outcome: Some("success".to_string()),
            }]
        );
    }

    /// Returns one representative public error for every failure classification.
    ///
    /// The match below is exhaustive so a new `ErrorClassification` variant forces this test
    /// to declare the expected completion level instead of silently skipping coverage.
    fn classification_cases() -> Vec<(ErrorClassification, PublicError)> {
        let empty = EmptyErrorParams {};
        let cases = vec![
            (
                ErrorClassification::Internal,
                PublicError::InternalError(empty),
            ),
            (
                ErrorClassification::Conflict,
                PublicError::ResourceInUse(empty),
            ),
            (
                ErrorClassification::InvalidRequest,
                PublicError::InvalidRequest(empty),
            ),
            (
                ErrorClassification::NotFound,
                PublicError::TaskNotFound(empty),
            ),
            (
                ErrorClassification::PayloadTooLarge,
                PublicError::ArchiveExpansionRatioExceeded(ora_contracts::EmptyErrorParams {}),
            ),
            (
                ErrorClassification::Unprocessable,
                PublicError::SkillManifestInvalid(empty),
            ),
        ];

        for (classification, _) in &cases {
            match classification {
                ErrorClassification::Internal
                | ErrorClassification::Conflict
                | ErrorClassification::InvalidRequest
                | ErrorClassification::NotFound
                | ErrorClassification::PayloadTooLarge
                | ErrorClassification::Unprocessable => {}
            }
        }

        cases
    }

    /// Maps each classification to the level documented in `docs/runtime-logging.md`.
    fn expected_level(classification: ErrorClassification) -> Level {
        match classification {
            ErrorClassification::Internal => Level::ERROR,
            ErrorClassification::Conflict => Level::WARN,
            ErrorClassification::InvalidRequest
            | ErrorClassification::NotFound
            | ErrorClassification::PayloadTooLarge
            | ErrorClassification::Unprocessable => Level::INFO,
        }
    }

    /// Returns the deterministic request identifier shared by lifecycle logging tests.
    fn test_request_id() -> RequestId {
        serde_json::from_str("\"550e8400-e29b-41d4-a716-446655440000\"").unwrap()
    }

    /// Describes one recorded event by the two facts completion assertions depend on.
    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Completion {
        level: Level,
        outcome: Option<String>,
    }

    /// Records emitted completions without depending on process-global subscriber state.
    #[derive(Clone, Debug, Default)]
    struct CompletionRecorder {
        completions: Arc<Mutex<Vec<Completion>>>,
    }

    impl CompletionRecorder {
        /// Builds the scoped subscriber layer used by one test.
        fn layer(&self) -> CompletionRecordingLayer {
            CompletionRecordingLayer {
                completions: self.completions.clone(),
            }
        }

        /// Returns captured completions in emission order.
        fn completions(&self) -> Vec<Completion> {
            self.completions.lock().unwrap().clone()
        }
    }

    /// Captures event metadata for assertions while leaving production formatting untouched.
    #[derive(Clone, Debug)]
    struct CompletionRecordingLayer {
        completions: Arc<Mutex<Vec<Completion>>>,
    }

    impl<S> Layer<S> for CompletionRecordingLayer
    where
        S: tracing::Subscriber,
    {
        /// Records each emitted event's level and outcome under the test-scoped TRACE subscriber.
        fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
            let mut visitor = OutcomeVisitor { outcome: None };
            event.record(&mut visitor);
            self.completions.lock().unwrap().push(Completion {
                level: *event.metadata().level(),
                outcome: visitor.outcome,
            });
        }
    }

    /// Extracts the single `outcome` field carried by completion events.
    struct OutcomeVisitor {
        outcome: Option<String>,
    }

    impl Visit for OutcomeVisitor {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "outcome" {
                self.outcome = Some(value.to_string());
            }
        }

        fn record_debug(&mut self, _field: &Field, _value: &dyn std::fmt::Debug) {}
    }
}
