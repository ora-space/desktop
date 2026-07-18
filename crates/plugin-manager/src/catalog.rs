use ora_plugin_protocol::{JsonSafeU64, PluginId, PluginKind, PluginPackageManifest};
use std::path::PathBuf;

/// A stable diagnostic category safe to expose without attacker-controlled payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginDiagnosticCode {
    InvalidManifest,
    UnsupportedSchemaVersion,
    UnsupportedPackageLayout,
    IncompatibleOra,
    IncompatiblePluginApi,
    IncompatibleBun,
    UnsupportedKind,
    MissingReceipt,
    InvalidReceipt,
    IntegrityMismatch,
    MissingStateRecord,
    MissingInstallFiles,
    PendingRemoval,
    UntrackedInstall,
    RecoveryRequired,
    UnsafeFilesystemObject,
    BudgetExceeded,
}

/// A bounded, structured management diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDiagnostic {
    pub code: PluginDiagnosticCode,
    pub message: String,
}

impl PluginDiagnostic {
    /// Constructs a diagnostic while preventing an unbounded filesystem/parser message.
    pub fn new(code: PluginDiagnosticCode, message: impl Into<String>) -> Self {
        let mut message = message.into();
        if message.len() > 4096 {
            message.truncate(4096);
        }
        Self { code, message }
    }
}

/// Separates schema validity from compatibility, support, and installed integrity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestValidity {
    Valid,
    Invalid,
}

/// Reports whether declared engine ranges include the current Host/runtime versions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeCompatibility {
    Compatible,
    Incompatible(CompatibilityReason),
}

/// The three independent engine axes used by Agent v1.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibilityReason {
    OraVersion,
    PluginApi,
    BunVersion,
}

/// Reports whether this Host implements an executor for a valid manifest shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeSupport {
    Supported,
    UnsupportedKind { kind: PluginKind },
    UnsupportedSchemaVersion { manifest_version: u64 },
}

/// Installed-copy integrity is orthogonal to manifest validity and runtime support.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrityStatus {
    NotApplicable,
    Verified,
    MissingReceipt,
    InvalidReceipt,
    DigestMismatch,
    StateMismatch,
}

/// Keeps every managed directory visible even when it is not eligible to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub plugin_id: Option<PluginId>,
    pub location: PathBuf,
    pub manifest: Option<PluginPackageManifest>,
    pub validity: ManifestValidity,
    pub compatibility: RuntimeCompatibility,
    pub support: RuntimeSupport,
    pub integrity: IntegrityStatus,
    pub diagnostics: Vec<PluginDiagnostic>,
}

/// An immutable, revisioned catalog projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCatalogSnapshot {
    pub revision: JsonSafeU64,
    pub entries: Vec<CatalogEntry>,
}
