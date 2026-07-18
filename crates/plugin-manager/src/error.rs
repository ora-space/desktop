use crate::{
    ActivationFailure, AgentContractFailure, CompatibilityReason, EffectiveDisableReason,
    HandshakeFailure, PluginDiagnostic, ProtocolFailure, TransportFailureStage,
    UnknownOutcomeCause,
};
use ora_plugin_protocol::{AgentBusinessErrorData, OperationId, PluginId, PluginKind};

/// Stable failures exposed by management and runtime facades.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum PluginError {
    #[error("plugin not found: {plugin_id}")]
    NotFound { plugin_id: PluginId },
    #[error("plugin already installed: {plugin_id}")]
    AlreadyInstalled { plugin_id: PluginId },
    #[error("plugin manifest is invalid")]
    InvalidManifest { diagnostics: Vec<PluginDiagnostic> },
    #[error("manifest schema version {manifest_version} is unsupported")]
    UnsupportedSchemaVersion { manifest_version: u64 },
    #[error("plugin package layout is unsupported")]
    UnsupportedPackageLayout { diagnostics: Vec<PluginDiagnostic> },
    #[error("plugin engine is incompatible: {reason:?}")]
    Incompatible { reason: CompatibilityReason },
    #[error("plugin kind is unsupported: {kind:?}")]
    UnsupportedKind { kind: PluginKind },
    #[error("installed plugin integrity mismatch: {plugin_id}")]
    IntegrityMismatch { plugin_id: PluginId },
    #[error("installed plugin files are missing: {plugin_id}")]
    MissingInstallFiles { plugin_id: PluginId },
    #[error("plugin is disabled: {plugin_id} ({reason:?})")]
    Disabled {
        plugin_id: PluginId,
        reason: EffectiveDisableReason,
    },
    #[error("plugin install conflicts with an existing managed object: {plugin_id}")]
    InstallConflict { plugin_id: PluginId },
    #[error("selection handle is invalid")]
    SelectionHandleInvalid { reason: AuthorizationHandleFailure },
    #[error("candidate handle is invalid")]
    CandidateHandleInvalid { reason: AuthorizationHandleFailure },
    #[error("destructive confirmation handle is invalid")]
    DestructiveConfirmationInvalid,
    #[error("candidate source changed after review")]
    SourceChanged { reason: SourceChangeReason },
    #[error("operation {operation_id} requires recovery")]
    RecoveryRequired {
        operation_id: OperationId,
        diagnostic: PluginDiagnostic,
    },
    #[error("plugin removal remains pending: {plugin_id}")]
    RemovalPending { plugin_id: PluginId },
    #[error("plugin state is corrupt")]
    StateCorrupt,
    #[error("plugin state schema version {schema_version} is unsupported")]
    StateVersionUnsupported { schema_version: u64 },
    #[error("persistence commit for {operation_id} is uncertain")]
    PersistenceUncertain { operation_id: OperationId },
    #[error("Ora data directory is already owned by another backend")]
    DataDirInUse,
    #[error("plugin runtime assets are unavailable")]
    PluginRuntimeUnavailable,
    #[error("plugin launch grant values are unavailable: {plugin_id}")]
    LaunchGrantUnavailable { plugin_id: PluginId },
    #[error("plugin launch grant schema is invalid")]
    InvalidLaunchGrant,
    #[error("plugin process tree containment is unavailable: {plugin_id}")]
    TreeKillUnavailable { plugin_id: PluginId },
    #[error("plugin process failed to start: {plugin_id}")]
    ProcessSpawnFailed { plugin_id: PluginId },
    #[error("plugin handshake failed: {plugin_id} ({reason:?})")]
    HandshakeFailed {
        plugin_id: PluginId,
        reason: HandshakeFailure,
    },
    #[error("plugin activation failed: {plugin_id} ({reason:?})")]
    ActivationFailed {
        plugin_id: PluginId,
        reason: ActivationFailure,
    },
    #[error("plugin deactivation failed: {plugin_id}")]
    DeactivationFailed { plugin_id: PluginId },
    #[error("plugin violated the wire protocol: {plugin_id} ({reason:?})")]
    ProtocolViolation {
        plugin_id: PluginId,
        reason: ProtocolFailure,
    },
    #[error("plugin process tree cleanup timed out: {plugin_id} generation {generation}")]
    TreeCleanupTimeout {
        plugin_id: PluginId,
        generation: u64,
    },
    #[error("plugin invocation exceeded consumer backpressure: {request_id}")]
    BackpressureExceeded {
        plugin_id: PluginId,
        request_id: String,
    },
    #[error("plugin returned an invalid Agent contract value: {request_id} ({reason:?})")]
    AgentContractViolation {
        plugin_id: PluginId,
        request_id: String,
        reason: AgentContractFailure,
    },
    #[error("plugin is busy: {request_id}")]
    PluginBusy {
        plugin_id: PluginId,
        request_id: String,
    },
    #[error("Agent provider returned a business failure: {request_id}")]
    AgentBusinessFailure {
        plugin_id: PluginId,
        request_id: String,
        message: String,
        data: AgentBusinessErrorData,
    },
    #[error("plugin transport failed for {request_id} at {stage:?}")]
    TransportFailed {
        plugin_id: PluginId,
        request_id: String,
        stage: TransportFailureStage,
    },
    #[error("plugin request timed out: {request_id}")]
    RequestTimedOut {
        plugin_id: PluginId,
        request_id: String,
    },
    #[error("plugin request was cancelled: {request_id}")]
    Cancelled {
        plugin_id: PluginId,
        request_id: String,
    },
    #[error("plugin process exited before completing the request")]
    PluginExited {
        plugin_id: PluginId,
        exit_code: Option<i32>,
    },
    #[error("plugin request outcome is unknown: {request_id} ({cause:?})")]
    UnknownOutcome {
        plugin_id: PluginId,
        request_id: String,
        cause: UnknownOutcomeCause,
    },
    #[error("backend is shutting down")]
    BackendShuttingDown,
    #[error("plugin operation failed: {message}")]
    Internal { message: String },
}

/// Internal authorization-handle reasons that adapters deliberately normalize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationHandleFailure {
    Unknown,
    Expired,
    WrongSession,
    WrongPurpose,
    AlreadyConsumed,
}

/// Stable source changes that force a fresh identify/review cycle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceChangeReason {
    RootMissing,
    RootIdentityMismatch,
    StagingValidationFailed,
    PluginIdentityMismatch,
    ContentDigestMismatch,
}
