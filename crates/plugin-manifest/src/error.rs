use crate::{PluginKindError, PluginNameError, PluginNamespaceError, Sha256DigestError, UrlError};
use ora_utils::GitBranchNameError;
use std::{fmt, ops::Range};
use thiserror::Error;

/// Reports structural and semantic failures while parsing one plugin release manifest.
#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("unsupported plugin manifest resolver {found}")]
    UnsupportedResolver { found: u64 },
    #[error("invalid TOML manifest: {source}")]
    InvalidToml {
        #[source]
        source: toml::de::Error,
        span: Option<Range<usize>>,
    },
    #[error("invalid manifest field {field}: {reason}")]
    InvalidField {
        field: ManifestField,
        reason: InvalidFieldReason,
    },
}

/// Identifies one semantic manifest field without requiring callers to parse dotted strings.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ManifestField {
    Name,
    Namespace,
    Kind,
    Version,
    Description,
    Homepage,
    License,
    Url,
    Sha256,
    HeadRepository,
    HeadBranch,
    DependenciesOra,
}

impl ManifestField {
    /// Returns the stable dotted manifest path for this field.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::Namespace => "namespace",
            Self::Kind => "kind",
            Self::Version => "version",
            Self::Description => "description",
            Self::Homepage => "homepage",
            Self::License => "license",
            Self::Url => "url",
            Self::Sha256 => "sha256",
            Self::HeadRepository => "head.repository",
            Self::HeadBranch => "head.branch",
            Self::DependenciesOra => "dependencies.ora",
        }
    }
}

impl fmt::Display for ManifestField {
    /// Writes the stable dotted manifest path.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Describes the semantic rule that rejected a structurally valid field.
#[derive(Debug, Error)]
pub enum InvalidFieldReason {
    #[error(transparent)]
    InvalidPluginName(#[from] PluginNameError),
    #[error(transparent)]
    InvalidNamespace(#[from] PluginNamespaceError),
    #[error(transparent)]
    InvalidKind(#[from] PluginKindError),
    #[error("invalid semantic version: {0}")]
    InvalidVersion(#[source] semver::Error),
    #[error("field must not be empty")]
    Empty,
    #[error("field exceeds {max_bytes} bytes: {actual_bytes}")]
    TooLong {
        max_bytes: usize,
        actual_bytes: usize,
    },
    #[error("field must not contain leading or trailing whitespace")]
    LeadingOrTrailingWhitespace,
    #[error("field must not contain control characters")]
    ContainsControlCharacter,
    #[error("field must contain ASCII text only")]
    NonAscii,
    #[error(transparent)]
    InvalidUrl(#[from] UrlError),
    #[error(transparent)]
    InvalidSha256(#[from] Sha256DigestError),
    #[error(transparent)]
    InvalidGitBranch(#[from] GitBranchNameError),
    #[error("invalid Ora version requirement: {0}")]
    InvalidVersionRequirement(#[source] semver::Error),
}
