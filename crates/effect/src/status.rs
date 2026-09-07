use crate::{
    ArtifactId, ConditionId, ConsumerIdentity, ConsumerRevisionId, DesiredEffectIdentity,
    EffectOperationId, EffectResourceId, EffectTargetId, FencingToken, Generation, ManagedIdentity,
    ProjectionDigest, ReconcileAttemptId, RetryAttempt, StatusVersion, WorkerIdentity,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Local wall-clock timestamp in milliseconds used for leases and scheduling.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct LocalTimestamp(i64);

impl LocalTimestamp {
    pub fn from_millis(value: i64) -> Self {
        Self(value)
    }

    /// Returns the persistence representation.
    pub fn millis(self) -> i64 {
        self.0
    }
}

/// Four evidence-backed generation watermarks for a Target.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetProgress {
    desired: Generation,
    observed: Generation,
    applied: Generation,
    ready: Generation,
}

impl TargetProgress {
    /// Restores progress only when every watermark satisfies the required partial order.
    pub fn restore(
        desired: Generation,
        observed: Generation,
        applied: Generation,
        ready: Generation,
    ) -> Result<Self, StatusTransitionError> {
        if ready > applied || applied > observed || observed > desired {
            return Err(StatusTransitionError::InvalidTargetWatermarks);
        }
        Ok(Self {
            desired,
            observed,
            applied,
            ready,
        })
    }

    /// Starts all watermarks at zero while recording the current Desired generation.
    pub fn pending(desired: Generation) -> Self {
        Self {
            desired,
            observed: Generation::default(),
            applied: Generation::default(),
            ready: Generation::default(),
        }
    }

    pub fn desired(self) -> Generation {
        self.desired
    }

    pub fn observed(self) -> Generation {
        self.observed
    }

    pub fn applied(self) -> Generation {
        self.applied
    }

    pub fn ready(self) -> Generation {
        self.ready
    }
}

/// Generic position inside one Target reconciliation attempt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconcileStage {
    Planning,
    Coordinating,
    Applying,
    Verifying,
    Activating,
}

/// Target phase contains only generic convergence position, never Consumer-specific state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "phase", content = "detail", rename_all = "snake_case")]
pub enum TargetPhase {
    Pending,
    Reconciling(ReconcileStage),
    Current,
    CurrentWithIssues,
    Retiring,
    RecoveryRequired(EffectOperationId),
}

/// Explicitly selects whether verified non-blocking Conditions remain on a Current Target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetIssueState {
    Clear,
    HasNonBlockingIssues,
}

/// Current evidence-backed convergence snapshot for one complete Target.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetStatus {
    target: EffectTargetId,
    progress: TargetProgress,
    phase: TargetPhase,
    version: StatusVersion,
}

impl TargetStatus {
    /// Creates the initial pending status for a newly declared Target.
    pub fn pending(target: EffectTargetId, desired: Generation) -> Self {
        Self {
            target,
            progress: TargetProgress::pending(desired),
            phase: TargetPhase::Pending,
            version: StatusVersion::initial(),
        }
    }

    /// Restores a persisted snapshot after validating its watermark invariant.
    pub fn restore(
        target: EffectTargetId,
        progress: TargetProgress,
        phase: TargetPhase,
        version: StatusVersion,
    ) -> Self {
        Self {
            target,
            progress,
            phase,
            version,
        }
    }

    pub fn target(&self) -> &EffectTargetId {
        &self.target
    }

    pub fn progress(&self) -> TargetProgress {
        self.progress
    }

    pub fn phase(&self) -> &TargetPhase {
        &self.phase
    }

    pub fn version(&self) -> StatusVersion {
        self.version
    }

    /// Raises Desired without manufacturing observation, application, or readiness evidence.
    pub fn request_generation(
        &mut self,
        generation: Generation,
    ) -> Result<(), StatusTransitionError> {
        if generation < self.progress.desired {
            return Err(StatusTransitionError::GenerationRegression);
        }
        self.progress.desired = generation;
        self.phase = TargetPhase::Pending;
        self.advance_version()
    }

    /// Records that planning evaluated the complete Desired snapshot for this generation.
    pub fn record_observed(&mut self, generation: Generation) -> Result<(), StatusTransitionError> {
        if generation < self.progress.observed || generation > self.progress.desired {
            return Err(StatusTransitionError::InvalidObservedGeneration);
        }
        self.progress.observed = generation;
        self.phase = TargetPhase::Reconciling(ReconcileStage::Planning);
        self.advance_version()
    }

    /// Records that every Resource required by the Target projection was verified.
    pub fn record_applied(&mut self, generation: Generation) -> Result<(), StatusTransitionError> {
        if generation < self.progress.applied || generation > self.progress.observed {
            return Err(StatusTransitionError::InvalidAppliedGeneration);
        }
        self.progress.applied = generation;
        self.phase = TargetPhase::Reconciling(ReconcileStage::Activating);
        self.advance_version()
    }

    /// Advances readiness only with a receipt matching the exact Target projection inputs.
    pub fn record_ready(
        &mut self,
        receipt: &ReadinessReceipt,
        expected_consumer_revision: &ConsumerRevisionId,
        expected_projection: &ProjectionDigest,
        issue_state: TargetIssueState,
    ) -> Result<(), StatusTransitionError> {
        if receipt.target != self.target
            || receipt.consumer_revision != *expected_consumer_revision
            || receipt.projection != *expected_projection
            || receipt.generation > self.progress.applied
            || receipt.generation < self.progress.ready
        {
            return Err(StatusTransitionError::MismatchedReadinessReceipt);
        }
        self.progress.ready = receipt.generation;
        self.phase = match issue_state {
            TargetIssueState::Clear => TargetPhase::Current,
            TargetIssueState::HasNonBlockingIssues => TargetPhase::CurrentWithIssues,
        };
        self.advance_version()
    }

    /// Moves the Target into explicit manual recovery without erasing its proven watermarks.
    pub fn require_recovery(
        &mut self,
        operation: EffectOperationId,
    ) -> Result<(), StatusTransitionError> {
        self.phase = TargetPhase::RecoveryRequired(operation);
        self.advance_version()
    }

    /// Advances optimistic status version with every atomic domain transition.
    fn advance_version(&mut self) -> Result<(), StatusTransitionError> {
        self.version = self.version.next()?;
        Ok(())
    }
}

/// Resource phase has no readiness state because only a Consumer can prove consumption readiness.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "phase", content = "operation", rename_all = "snake_case")]
pub enum ResourcePhase {
    Pending,
    Reconciling,
    Current,
    Retiring,
    RecoveryRequired(EffectOperationId),
}

/// Independent materialization watermarks for one Resource.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceStatus {
    resource: EffectResourceId,
    desired: Generation,
    observed: Generation,
    applied: Generation,
    phase: ResourcePhase,
    version: StatusVersion,
}

impl ResourceStatus {
    /// Creates the initial status without pretending the Resource has been observed.
    pub fn pending(resource: EffectResourceId, desired: Generation) -> Self {
        Self {
            resource,
            desired,
            observed: Generation::default(),
            applied: Generation::default(),
            phase: ResourcePhase::Pending,
            version: StatusVersion::initial(),
        }
    }

    /// Restores persisted state only when materialization watermarks are ordered.
    pub fn restore(
        resource: EffectResourceId,
        desired: Generation,
        observed: Generation,
        applied: Generation,
        phase: ResourcePhase,
        version: StatusVersion,
    ) -> Result<Self, StatusTransitionError> {
        if applied > observed || observed > desired {
            return Err(StatusTransitionError::InvalidResourceWatermarks);
        }
        Ok(Self {
            resource,
            desired,
            observed,
            applied,
            phase,
            version,
        })
    }

    pub fn resource(&self) -> &EffectResourceId {
        &self.resource
    }

    pub fn desired(&self) -> Generation {
        self.desired
    }

    pub fn observed(&self) -> Generation {
        self.observed
    }

    pub fn applied(&self) -> Generation {
        self.applied
    }

    pub fn phase(&self) -> &ResourcePhase {
        &self.phase
    }

    pub fn version(&self) -> StatusVersion {
        self.version
    }

    /// Raises the Resource Desired watermark without changing observed or applied evidence.
    pub fn request_generation(
        &mut self,
        generation: Generation,
    ) -> Result<(), StatusTransitionError> {
        if generation < self.desired {
            return Err(StatusTransitionError::GenerationRegression);
        }
        self.desired = generation;
        self.phase = ResourcePhase::Pending;
        self.advance_version()
    }

    /// Records a complete adapter observation used to plan this generation.
    pub fn record_observed(&mut self, generation: Generation) -> Result<(), StatusTransitionError> {
        if generation < self.observed || generation > self.desired {
            return Err(StatusTransitionError::InvalidObservedGeneration);
        }
        self.observed = generation;
        self.phase = ResourcePhase::Reconciling;
        self.advance_version()
    }

    /// Records exact verification of the complete Resource projection.
    pub fn record_applied(&mut self, generation: Generation) -> Result<(), StatusTransitionError> {
        if generation < self.applied || generation > self.observed {
            return Err(StatusTransitionError::InvalidAppliedGeneration);
        }
        self.applied = generation;
        self.phase = ResourcePhase::Current;
        self.advance_version()
    }

    /// Preserves manual recovery authority without changing proven watermarks.
    pub fn require_recovery(
        &mut self,
        operation: EffectOperationId,
    ) -> Result<(), StatusTransitionError> {
        self.phase = ResourcePhase::RecoveryRequired(operation);
        self.advance_version()
    }

    /// Advances optimistic Resource status version with one atomic domain transition.
    fn advance_version(&mut self) -> Result<(), StatusTransitionError> {
        self.version = self.version.next()?;
        Ok(())
    }
}

/// Owner of a current Condition and therefore the status it may block.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "identity", rename_all = "snake_case")]
pub enum ConditionOwner {
    Target(EffectTargetId),
    Resource(EffectResourceId),
}

/// Exact domain subject explained by a Condition.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "identity", rename_all = "snake_case")]
pub enum ConditionSubject {
    Consumer(ConsumerIdentity),
    Target(EffectTargetId),
    DesiredEffect(DesiredEffectIdentity),
    Resource(EffectResourceId),
    ManagedItem(ManagedIdentity),
    Operation(EffectOperationId),
    Artifact(ArtifactId),
}

/// Stable code used by control flow instead of parsing human messages.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct StableConditionCode(String);

impl StableConditionCode {
    /// Constructs a code from a trusted non-empty static literal owned by a planner or Core.
    pub fn from_static(value: &'static str) -> Self {
        assert!(
            !value.trim().is_empty(),
            "a static Effect Condition code must not be empty"
        );
        Self(value.to_string())
    }

    /// Refuses empty codes because retry and UI logic require stable classification.
    pub fn parse(value: impl Into<String>) -> Result<Self, StatusTransitionError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(StatusTransitionError::EmptyConditionCode);
        }
        Ok(Self(value))
    }

    /// Returns the stable machine-readable code.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Determines whether a current fact blocks its owner's watermark progression.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionImpact {
    Blocking,
    NonBlocking,
}

/// Versioned exponential retry parameters for a transient condition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BackoffPolicy {
    pub initial_delay_millis: u64,
    pub maximum_delay_millis: u64,
}

/// Resume strategy is an explicit state instead of an ambiguous retry flag.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "policy", rename_all = "snake_case")]
pub enum ConditionRetry {
    OnChange,
    Backoff(BackoffPolicy),
    Manual,
}

/// Associates a Condition either with a precise generation or with unscoped runtime state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "generation", rename_all = "snake_case")]
pub enum ConditionGeneration {
    Unscoped,
    At(Generation),
}

/// Safe explanatory fields that exclude raw configuration, paths, and error chains by contract.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SafeConditionDetails {
    pub message: String,
    pub parameters: BTreeMap<String, String>,
}

/// Current structured fact used by control flow and user interfaces.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectCondition {
    pub identity: ConditionId,
    pub owner: ConditionOwner,
    pub subject: ConditionSubject,
    pub code: StableConditionCode,
    pub impact: ConditionImpact,
    pub retry: ConditionRetry,
    pub generation: ConditionGeneration,
    pub safe_details: SafeConditionDetails,
    pub first_observed_at: LocalTimestamp,
    pub last_observed_at: LocalTimestamp,
}

/// Deterministic planner output that persistence can merge into current Condition identity/time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConditionProposal {
    pub owner: ConditionOwner,
    pub subject: ConditionSubject,
    pub code: StableConditionCode,
    pub impact: ConditionImpact,
    pub retry: ConditionRetry,
    pub generation: ConditionGeneration,
    pub safe_details: SafeConditionDetails,
}

/// Diagnostic-only reasons that coalesce into a level-triggered request.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeReason {
    DesiredChanged,
    ConsumerRevisionChanged,
    DeclarationChanged,
    ResourceChanged,
    LeaseExpired,
    RetryDue,
    UserRequested,
    TargetRetiring,
}

/// Versioned trigger that explains which change may resume a blocked request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResumeTrigger {
    pub version: u32,
    pub payload: Value,
}

/// Claim proving which worker currently owns one Target reconcile.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReconcileClaim {
    pub token: FencingToken,
    pub worker: WorkerIdentity,
    pub lease_until: LocalTimestamp,
}

/// Illegal combinations of claim, retry, and block fields are excluded by the tagged state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", content = "detail", rename_all = "snake_case")]
pub enum ReconcileRequestState {
    Pending,
    Claimed(ReconcileClaim),
    RetryScheduled {
        attempt: RetryAttempt,
        not_before: LocalTimestamp,
    },
    Blocked {
        conditions: BTreeSet<ConditionId>,
        resume: ResumeTrigger,
    },
}

/// Durable level-triggered fact that a Target must be evaluated again.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReconcileRequest {
    pub target: EffectTargetId,
    pub requested_generation: Generation,
    pub state: ReconcileRequestState,
    pub wake_reasons: BTreeSet<WakeReason>,
}

/// Independent fenced lease authorizing mutation of one potentially shared Resource.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceClaim {
    pub resource: EffectResourceId,
    pub target_claim: FencingToken,
    pub resource_fence: FencingToken,
    pub worker: WorkerIdentity,
    pub lease_until: LocalTimestamp,
}

/// Versioned proof supplied by an adapter and interpreted only by that adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AdapterReceipt {
    pub version: u32,
    pub payload: Value,
}

/// Exact Consumer confirmation that one Target can consume its projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReadinessReceipt {
    pub target: EffectTargetId,
    pub generation: Generation,
    pub consumer_revision: ConsumerRevisionId,
    pub projection: ProjectionDigest,
    pub proof: AdapterReceipt,
}

/// Reports a rejected atomic status transition.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum StatusTransitionError {
    #[error("Target watermarks violate ready <= applied <= observed <= desired")]
    InvalidTargetWatermarks,
    #[error("Resource watermarks violate applied <= observed <= desired")]
    InvalidResourceWatermarks,
    #[error("a generation watermark cannot regress")]
    GenerationRegression,
    #[error("observed generation lacks matching Desired evidence")]
    InvalidObservedGeneration,
    #[error("applied generation lacks matching observation evidence")]
    InvalidAppliedGeneration,
    #[error("readiness receipt does not match the current Target projection")]
    MismatchedReadinessReceipt,
    #[error("condition code must not be empty")]
    EmptyConditionCode,
    #[error(transparent)]
    Identity(#[from] crate::IdentityError),
}

/// Connects a receipt to its immutable reconcile attempt when persisted by the repository.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AttemptReadinessReceipt {
    pub attempt: ReconcileAttemptId,
    pub receipt: ReadinessReceipt,
}
