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
            duration_ms = self.duration_ms(),
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
            duration_ms = self.duration_ms(),
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
                duration_ms = self.duration_ms(),
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
                duration_ms = self.duration_ms(),
                error.code = code,
                error.message = report.message(),
                error.chain = report.chain(),
                error.chain_depth = report.chain_depth(),
                "request completed"
            ),
            ErrorClassification::InvalidRequest | ErrorClassification::NotFound => ora_info!(
                operation = self.inner.operation.as_ref(),
                request_id = %self.inner.request_id,
                outcome = "failure",
                duration_ms = self.duration_ms(),
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
            duration_ms = self.duration_ms(),
            "request completed"
        );
    }

    fn claim_completion(&self) -> bool {
        self.inner
            .completed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn duration_ms(&self) -> u64 {
        self.inner.started_at.elapsed().as_millis() as u64
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
    use tracing_subscriber::layer::{Context, Layer};

    struct FixedRequestIdGenerator(RequestId);

    impl RequestIdGenerator for FixedRequestIdGenerator {
        fn generate(&self) -> RequestId {
            self.0
        }
    }

    #[test]
    fn cloned_lifecycles_share_the_id_and_one_completion_claim() {
        let request_id: RequestId =
            serde_json::from_str("\"550e8400-e29b-41d4-a716-446655440000\"").unwrap();
        let lifecycle =
            RequestLifecycle::start("test_operation", &FixedRequestIdGenerator(request_id));
        let cloned = lifecycle.clone();

        assert_eq!(lifecycle.request_id(), request_id);
        assert_eq!(cloned.request_id(), request_id);
        assert!(lifecycle.claim_completion());
        assert!(!cloned.claim_completion());
    }

    /// Verifies every backend classification emits its documented completion level.
    #[test]
    fn failure_classifications_map_to_expected_log_levels() {
        let recorder = LevelRecorder::default();
        with_recorded_trace_logging(recorder.layer(), || {
            let cases = [
                (
                    ErrorClassification::Internal,
                    PublicError::InternalError(EmptyErrorParams {}),
                ),
                (
                    ErrorClassification::Conflict,
                    PublicError::ResourceInUse(EmptyErrorParams {}),
                ),
                (
                    ErrorClassification::InvalidRequest,
                    PublicError::InvalidRequest(EmptyErrorParams {}),
                ),
                (
                    ErrorClassification::NotFound,
                    PublicError::TaskNotFound(EmptyErrorParams {}),
                ),
            ];

            for (classification, public_error) in cases {
                let lifecycle = RequestLifecycle::start(
                    "test_operation",
                    &FixedRequestIdGenerator(test_request_id()),
                );
                lifecycle.complete_failure(&BackendError::new(
                    classification,
                    public_error,
                    "test failure",
                ));
            }
        });

        assert_eq!(
            recorder.levels(),
            vec![Level::ERROR, Level::WARN, Level::INFO, Level::INFO]
        );
    }

    /// Returns the deterministic request identifier shared by lifecycle logging tests.
    fn test_request_id() -> RequestId {
        serde_json::from_str("\"550e8400-e29b-41d4-a716-446655440000\"").unwrap()
    }

    /// Records emitted levels without depending on process-global subscriber state.
    #[derive(Clone, Debug, Default)]
    struct LevelRecorder {
        levels: Arc<Mutex<Vec<Level>>>,
    }

    impl LevelRecorder {
        /// Builds the scoped subscriber layer used by one test.
        fn layer(&self) -> LevelRecordingLayer {
            LevelRecordingLayer {
                levels: self.levels.clone(),
            }
        }

        /// Returns captured event levels in emission order.
        fn levels(&self) -> Vec<Level> {
            self.levels.lock().unwrap().clone()
        }
    }

    /// Captures event metadata for assertions while leaving production formatting untouched.
    #[derive(Clone, Debug)]
    struct LevelRecordingLayer {
        levels: Arc<Mutex<Vec<Level>>>,
    }

    impl<S> Layer<S> for LevelRecordingLayer
    where
        S: tracing::Subscriber,
    {
        /// Records each emitted event's level under the test-scoped TRACE subscriber.
        fn on_event(&self, event: &tracing::Event<'_>, _context: Context<'_, S>) {
            self.levels.lock().unwrap().push(*event.metadata().level());
        }
    }
}
