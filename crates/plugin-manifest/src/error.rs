use crate::{
    DownloadActionError, MethodNameError, PathPrefixError, PluginKind, PluginKindError,
    PluginNameError, PluginNamespaceError, Sha256DigestError, UrlError, WebviewUrlError,
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
    /// attribute the failure to one (`webview.downloads.rules[0].page`), so callers can report
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
    /// The human-readable display title, distinct from the identifier it falls back to.
    Title,
    /// The installed package's name segment, spelled `identifier` in the `orax.toml` shipped
    /// inside a package (distinct from the marketplace release form's `name` field).
    Identifier,
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
    /// The whole `[workbench]` section, used when its presence disagrees with `kind`.
    Workbench,
    /// The `workbench.methods` array as a whole.
    WorkbenchMethods,
    /// The method at `index` in `workbench.methods`.
    WorkbenchMethod {
        index: usize,
    },
    /// The whole `[webview]` section, used when its presence disagrees with `kind`.
    Webview,
    WebviewStartUrl,
    /// The `webview.allowed_origins` array as a whole.
    WebviewAllowedOrigins,
    /// The origin at `index` in `webview.allowed_origins`.
    WebviewAllowedOrigin {
        index: usize,
    },
    WebviewDownloadsFallback,
    /// One field of the rule at `index` in `webview.downloads.rules`.
    WebviewDownloadRule {
        index: usize,
        field: RuleField,
    },
}

impl fmt::Display for ManifestField {
    /// Writes the stable dotted manifest path, with array indices in brackets.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Title => formatter.write_str("title"),
            Self::Identifier => formatter.write_str("identifier"),
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
            Self::Workbench => formatter.write_str("workbench"),
            Self::WorkbenchMethods => formatter.write_str("workbench.methods"),
            Self::WorkbenchMethod { index } => write!(formatter, "workbench.methods[{index}]"),
            Self::Webview => formatter.write_str("webview"),
            Self::WebviewStartUrl => formatter.write_str("webview.start_url"),
            Self::WebviewAllowedOrigins => formatter.write_str("webview.allowed_origins"),
            Self::WebviewAllowedOrigin { index } => {
                write!(formatter, "webview.allowed_origins[{index}]")
            }
            Self::WebviewDownloadsFallback => formatter.write_str("webview.downloads.fallback"),
            Self::WebviewDownloadRule { index, field } => {
                write!(formatter, "webview.downloads.rules[{index}].{field}")
            }
        }
    }
}

/// Identifies one field of a `[[webview.downloads.rules]]` entry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum RuleField {
    PageOrigin,
    PagePathPrefix,
    Action,
}

impl fmt::Display for RuleField {
    /// Writes the field path relative to its rule entry.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::PageOrigin => "page.origin",
            Self::PagePathPrefix => "page.path_prefix",
            Self::Action => "action",
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
    #[error("value is declared more than once")]
    Duplicate,
    #[error("download action `{action}` requires user interaction and cannot run automatically")]
    NonAutomatableDownloadAction { action: String },
    #[error(transparent)]
    InvalidMethodName(#[from] MethodNameError),
    #[error(transparent)]
    InvalidWebviewUrl(#[from] WebviewUrlError),
    #[error(transparent)]
    InvalidPathPrefix(#[from] PathPrefixError),
    #[error(transparent)]
    InvalidDownloadAction(DownloadActionError),
    #[error("action must declare exactly one of `auto`, `prompt`, or `reject = true`")]
    AmbiguousDownloadAction,
}
