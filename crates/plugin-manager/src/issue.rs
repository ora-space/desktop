use std::fmt;
use std::path::{Path, PathBuf};

/// Categorizes one non-fatal plugin discovery problem for structured reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginDiscoveryIssueKind {
    RootUnreadable,
    EntryUnreadable,
    MissingManifest,
    ManifestNotFile,
    ManifestTooLarge,
    ManifestUnreadable,
    InvalidToml,
    InvalidManifest,
    DuplicatePluginId,
    UnusableLogo,
}

impl PluginDiscoveryIssueKind {
    /// Returns a stable machine-readable label suitable for structured logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RootUnreadable => "root_unreadable",
            Self::EntryUnreadable => "entry_unreadable",
            Self::MissingManifest => "missing_manifest",
            Self::ManifestNotFile => "manifest_not_file",
            Self::ManifestTooLarge => "manifest_too_large",
            Self::ManifestUnreadable => "manifest_unreadable",
            Self::InvalidToml => "invalid_toml",
            Self::InvalidManifest => "invalid_manifest",
            Self::DuplicatePluginId => "duplicate_plugin_id",
            Self::UnusableLogo => "unusable_logo",
        }
    }
}

/// Describes one skipped package or discovery operation without aborting the snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginDiscoveryIssue {
    path: PathBuf,
    kind: PluginDiscoveryIssueKind,
    field_path: Option<String>,
    message: String,
}

impl PluginDiscoveryIssue {
    /// Creates one issue with an optional manifest field path.
    pub(crate) fn new(
        path: PathBuf,
        kind: PluginDiscoveryIssueKind,
        field_path: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path,
            kind,
            field_path,
            message: message.into(),
        }
    }

    /// Returns the filesystem location associated with the issue.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the stable category of the issue.
    pub fn kind(&self) -> PluginDiscoveryIssueKind {
        self.kind
    }

    /// Returns the nested manifest field that failed, when one is available.
    pub fn field_path(&self) -> Option<&str> {
        self.field_path.as_deref()
    }

    /// Returns the human-readable cause supplied by the filesystem or validator.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for PluginDiscoveryIssue {
    /// Formats a concise issue description without hiding its structured fields.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.field_path {
            Some(field_path) => write!(
                formatter,
                "{} at {}: {}",
                self.kind.as_str(),
                field_path,
                self.message
            ),
            None => write!(formatter, "{}: {}", self.kind.as_str(), self.message),
        }
    }
}

impl std::error::Error for PluginDiscoveryIssue {}
