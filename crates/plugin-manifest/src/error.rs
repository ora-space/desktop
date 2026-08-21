use crate::{
    PluginKind, PluginKindError, PluginNameError, PluginNamespaceError, Sha256DigestError,
    SurfaceInstancesError, UrlError, WebDataModeError,
};
use ora_utils::{GitBranchNameError, SlugError};
use std::{fmt, ops::Range};
use thiserror::Error;

/// Reports structural and semantic failures while parsing one plugin manifest.
#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("unsupported plugin manifest resolver {found}")]
    UnsupportedResolver { found: u64 },
    /// `path` is the dotted TOML path of the offending value when the deserializer could
    /// attribute the failure to one (`ui.surfaces[0].source.root`), so callers can report
    /// nested structural errors as precisely as semantic ones. The TOML error is boxed because
    /// it dominates the size of every `Result` in the crate.
    #[error("invalid TOML manifest: {source}")]
    InvalidToml {
        #[source]
        source: Box<toml::de::Error>,
        span: Option<Range<usize>>,
        path: Option<String>,
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
    /// The whole `[ui]` section, used when its presence disagrees with `kind`.
    Ui,
    /// The `ui.surfaces` array as a whole.
    UiSurfaces,
    /// One field of the surface at `index` in `ui.surfaces`.
    UiSurface {
        index: usize,
        field: SurfaceField,
    },
}

impl fmt::Display for ManifestField {
    /// Writes the stable dotted manifest path, with array indices in brackets.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Name => formatter.write_str("name"),
            Self::Namespace => formatter.write_str("namespace"),
            Self::Kind => formatter.write_str("kind"),
            Self::Version => formatter.write_str("version"),
            Self::Description => formatter.write_str("description"),
            Self::Homepage => formatter.write_str("homepage"),
            Self::License => formatter.write_str("license"),
            Self::Url => formatter.write_str("url"),
            Self::Sha256 => formatter.write_str("sha256"),
            Self::HeadRepository => formatter.write_str("head.repository"),
            Self::HeadBranch => formatter.write_str("head.branch"),
            Self::DependenciesOra => formatter.write_str("dependencies.ora"),
            Self::Ui => formatter.write_str("ui"),
            Self::UiSurfaces => formatter.write_str("ui.surfaces"),
            Self::UiSurface { index, field } => write!(formatter, "ui.surfaces[{index}].{field}"),
        }
    }
}

/// Identifies one field of a `[[ui.surfaces]]` entry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SurfaceField {
    Id,
    Title,
    Instances,
    WebDataMode,
}

impl fmt::Display for SurfaceField {
    /// Writes the field path relative to its surface entry.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Id => "id",
            Self::Title => "title",
            Self::Instances => "instances",
            Self::WebDataMode => "web_data.mode",
        })
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
    #[error("section is required for plugin kind `{kind}`")]
    MissingForKind { kind: PluginKind },
    #[error("section is not allowed for plugin kind `{kind}`")]
    NotAllowedForKind { kind: PluginKind },
    #[error("invalid slug: {0}")]
    InvalidSlug(#[from] SlugError),
    #[error(transparent)]
    InvalidSurfaceInstances(#[from] SurfaceInstancesError),
    #[error(transparent)]
    InvalidWebDataMode(#[from] WebDataModeError),
}
