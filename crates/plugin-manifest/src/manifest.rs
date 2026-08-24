use crate::webview::RawWebview;
use crate::workbench::RawWorkbench;
use crate::{
    HomepageUrl, InvalidFieldReason, ManifestError, ManifestField, PluginKind, PluginName,
    PluginNamespace, PluginWebview, PluginWorkbench, ReleaseUrl, RepositoryUrl, Sha256Digest,
};
use ora_utils::GitBranchName;
use semver::{Version, VersionReq};
use serde::Deserialize;
use std::str::FromStr;

const SUPPORTED_RESOLVER: u64 = 1;

/// Holds one fully validated plugin release manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginManifest {
    pub(crate) resolver: u64,
    pub(crate) name: PluginName,
    pub(crate) namespace: PluginNamespace,
    pub(crate) kind: PluginKind,
    pub(crate) version: Version,
    pub(crate) description: String,
    pub(crate) homepage: Option<HomepageUrl>,
    pub(crate) license: Option<String>,
    pub(crate) url: Option<ReleaseUrl>,
    pub(crate) sha256: Option<Sha256Digest>,
    pub(crate) head: Option<PluginHead>,
    pub(crate) dependencies: Option<PluginDependencies>,
    pub(crate) workbench: Option<PluginWorkbench>,
    pub(crate) webview: Option<PluginWebview>,
}

impl PluginManifest {
    /// Parses and validates one plugin release manifest from TOML text.
    ///
    /// A release manifest is the marketplace form and must declare the download metadata
    /// (`resolver`, `url`, `sha256`) needed to fetch and verify the package.
    pub fn parse(source: &str) -> Result<Self, ManifestError> {
        let raw: RawPluginManifest = deserialize(source)?;
        let (metadata, resolver, url, sha256) = raw.into_parts();
        Self::from_raw_parts(metadata, resolver, Some(url), Some(sha256))
    }

    /// Parses and validates an installed plugin's manifest (the `orax.toml` shipped inside a
    /// package). Installed manifests carry descriptive metadata only; the download-only `url` and
    /// `sha256` fields are optional, and an omitted `resolver` is accepted as the current version.
    pub fn parse_installed(source: &str) -> Result<Self, ManifestError> {
        let raw: RawInstalledManifest = deserialize(source)?;
        let (metadata, resolver, url, sha256) = raw.into_parts();
        let resolver = resolver.unwrap_or(SUPPORTED_RESOLVER);
        Self::from_raw_parts(metadata, resolver, url, sha256)
    }

    /// Applies every semantic validation rule to the values shared by both manifest forms,
    /// keeping the release and installed schemas on one validated domain model.
    fn from_raw_parts(
        metadata: RawMetadata,
        resolver: u64,
        url: Option<String>,
        sha256: Option<String>,
    ) -> Result<Self, ManifestError> {
        if resolver != SUPPORTED_RESOLVER {
            return Err(ManifestError::UnsupportedResolver { found: resolver });
        }

        // Keep semantic conversion explicit so the first error follows schema declaration order.
        let name = PluginName::parse(&metadata.name)
            .map_err(|reason| invalid_field(ManifestField::Name, reason.into()))?;
        let namespace = PluginNamespace::from_str(&metadata.namespace)
            .map_err(|reason| invalid_field(ManifestField::Namespace, reason.into()))?;
        let kind = PluginKind::from_str(&metadata.kind)
            .map_err(|reason| invalid_field(ManifestField::Kind, reason.into()))?;
        let version = Version::parse(&metadata.version).map_err(|reason| {
            invalid_field(
                ManifestField::Version,
                InvalidFieldReason::InvalidVersion(reason),
            )
        })?;
        validate_text(&metadata.description, TextPolicy::Description)
            .map_err(|reason| invalid_field(ManifestField::Description, reason))?;
        let homepage = metadata
            .homepage
            .as_deref()
            .map(HomepageUrl::parse)
            .transpose()
            .map_err(|reason| invalid_field(ManifestField::Homepage, reason.into()))?;
        if let Some(license) = metadata.license.as_deref() {
            validate_text(license, TextPolicy::License)
                .map_err(|reason| invalid_field(ManifestField::License, reason))?;
        }
        let url = url
            .map(|value| ReleaseUrl::parse(&value))
            .transpose()
            .map_err(|reason| invalid_field(ManifestField::Url, reason.into()))?;
        let sha256 = sha256
            .map(|value| Sha256Digest::parse(&value))
            .transpose()
            .map_err(|reason| invalid_field(ManifestField::Sha256, reason.into()))?;

        let head = metadata.head.map(PluginHead::try_from).transpose()?;
        let dependencies = metadata
            .dependencies
            .and_then(|dependencies| dependencies.ora)
            .map(|requirement| {
                VersionReq::parse(&requirement)
                    .map(|ora| PluginDependencies { ora })
                    .map_err(|reason| {
                        invalid_field(
                            ManifestField::DependenciesOra,
                            InvalidFieldReason::InvalidVersionRequirement(reason),
                        )
                    })
            })
            .transpose()?;
        let (workbench, webview) =
            validate_kind_sections(kind, metadata.workbench, metadata.webview)?;

        Ok(Self {
            resolver,
            name,
            namespace,
            kind,
            version,
            description: metadata.description,
            homepage,
            license: metadata.license,
            url,
            sha256,
            head,
            dependencies,
            workbench,
            webview,
        })
    }

    /// Returns the manifest resolver version.
    pub fn resolver(&self) -> u64 {
        self.resolver
    }

    /// Returns the complete plugin identifier.
    pub fn name(&self) -> &PluginName {
        &self.name
    }

    /// Returns the plugin source namespace.
    pub fn namespace(&self) -> PluginNamespace {
        self.namespace
    }

    /// Returns the plugin kind.
    pub fn kind(&self) -> PluginKind {
        self.kind
    }

    /// Returns the published semantic version.
    pub fn version(&self) -> &Version {
        &self.version
    }

    /// Returns the validated plugin description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the optional plugin homepage.
    pub fn homepage(&self) -> Option<&HomepageUrl> {
        self.homepage.as_ref()
    }

    /// Returns the optional unvalidated-as-SPDX license text.
    pub fn license(&self) -> Option<&str> {
        self.license.as_deref()
    }

    /// Returns the release package URL when this manifest declares one.
    pub fn url(&self) -> Option<&ReleaseUrl> {
        self.url.as_ref()
    }

    /// Returns the release package SHA-256 digest when this manifest declares one.
    pub fn sha256(&self) -> Option<&Sha256Digest> {
        self.sha256.as_ref()
    }

    /// Returns the download metadata for a marketplace release manifest.
    pub fn release(&self) -> Option<(&ReleaseUrl, &Sha256Digest)> {
        match (self.url.as_ref(), self.sha256.as_ref()) {
            (Some(url), Some(sha256)) => Some((url, sha256)),
            _ => None,
        }
    }

    /// Returns optional source repository metadata.
    pub fn head(&self) -> Option<&PluginHead> {
        self.head.as_ref()
    }

    /// Returns the optional declared host dependency.
    pub fn dependencies(&self) -> Option<&PluginDependencies> {
        self.dependencies.as_ref()
    }

    /// Returns the `[workbench]` section; only a workbench-kind manifest may carry one.
    ///
    /// The section is optional even for that kind: a workbench plugin without page-callable
    /// methods (a purely static page) simply omits it.
    pub fn workbench(&self) -> Option<&PluginWorkbench> {
        self.workbench.as_ref()
    }

    /// Returns the `[webview]` section, present exactly when `kind` is [`PluginKind::Webview`].
    pub fn webview(&self) -> Option<&PluginWebview> {
        self.webview.as_ref()
    }
}

/// Deserializes one manifest form, keeping the TOML path of a structural failure.
///
/// `serde_path_to_error` is used instead of `toml::from_str` because nested sections such as
/// `[[webview.downloads.rules]]` would otherwise report "unknown field" without saying which
/// entry.
fn deserialize<'de, T: Deserialize<'de>>(source: &'de str) -> Result<T, ManifestError> {
    let deserializer = toml::de::Deserializer::parse(source).map_err(|source| {
        let span = source.span();
        ManifestError::InvalidToml {
            source: Box::new(source),
            span,
            path: None,
        }
    })?;
    serde_path_to_error::deserialize(deserializer).map_err(|error| {
        let path = error.path().to_string();
        let source = error.into_inner();
        let span = source.span();
        ManifestError::InvalidToml {
            source: Box::new(source),
            span,
            // The root path renders as "." which carries no information.
            path: (path != ".").then_some(path),
        }
    })
}

/// Pairs `kind` with the sections it may carry so a manifest cannot be half of two kinds.
///
/// `[webview]` is required by, and exclusive to, `kind = "webview"`; `[workbench]` is exclusive
/// to `kind = "workbench"` but optional there, because a static page needs no methods.
fn validate_kind_sections(
    kind: PluginKind,
    workbench: Option<RawWorkbench>,
    webview: Option<RawWebview>,
) -> Result<(Option<PluginWorkbench>, Option<PluginWebview>), ManifestError> {
    let workbench = match (kind, workbench) {
        (PluginKind::Workbench, Some(workbench)) => Some(PluginWorkbench::try_from(workbench)?),
        (PluginKind::Workbench, None) => None,
        (PluginKind::Agent | PluginKind::Webview, Some(_)) => {
            return Err(invalid_field(
                ManifestField::Workbench,
                InvalidFieldReason::NotAllowedForKind { kind },
            ));
        }
        (PluginKind::Agent | PluginKind::Webview, None) => None,
    };
    let webview = match (kind, webview) {
        (PluginKind::Webview, Some(webview)) => Some(PluginWebview::try_from(webview)?),
        (PluginKind::Webview, None) => {
            return Err(invalid_field(
                ManifestField::Webview,
                InvalidFieldReason::MissingForKind { kind },
            ));
        }
        (PluginKind::Agent | PluginKind::Workbench, Some(_)) => {
            return Err(invalid_field(
                ManifestField::Webview,
                InvalidFieldReason::NotAllowedForKind { kind },
            ));
        }
        (PluginKind::Agent | PluginKind::Workbench, None) => None,
    };

    Ok((workbench, webview))
}

/// Holds validated source repository metadata for one plugin release.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginHead {
    pub(crate) repository: RepositoryUrl,
    pub(crate) branch: GitBranchName,
}

impl PluginHead {
    /// Returns the source repository URL.
    pub fn repository(&self) -> &RepositoryUrl {
        &self.repository
    }

    /// Returns the source repository branch.
    pub fn branch(&self) -> &GitBranchName {
        &self.branch
    }
}

impl TryFrom<RawHead> for PluginHead {
    type Error = ManifestError;

    /// Converts source metadata after applying repository and branch policies in field order.
    fn try_from(raw: RawHead) -> Result<Self, Self::Error> {
        let repository = RepositoryUrl::parse(&raw.repository)
            .map_err(|reason| invalid_field(ManifestField::HeadRepository, reason.into()))?;
        let branch = GitBranchName::parse(&raw.branch)
            .map_err(|reason| invalid_field(ManifestField::HeadBranch, reason.into()))?;

        Ok(Self { repository, branch })
    }
}

/// Holds the declared Ora host version requirement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginDependencies {
    pub(crate) ora: VersionReq,
}

impl PluginDependencies {
    /// Returns the declared Ora host version requirement.
    pub fn ora(&self) -> &VersionReq {
        &self.ora
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPluginManifest {
    resolver: u64,
    name: String,
    namespace: String,
    kind: String,
    version: String,
    description: String,
    homepage: Option<String>,
    license: Option<String>,
    url: String,
    sha256: String,
    head: Option<RawHead>,
    dependencies: Option<RawDependencies>,
    workbench: Option<RawWorkbench>,
    webview: Option<RawWebview>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInstalledManifest {
    resolver: Option<u64>,
    name: String,
    namespace: String,
    kind: String,
    version: String,
    description: String,
    homepage: Option<String>,
    license: Option<String>,
    url: Option<String>,
    sha256: Option<String>,
    head: Option<RawHead>,
    dependencies: Option<RawDependencies>,
    workbench: Option<RawWorkbench>,
    webview: Option<RawWebview>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct RawHead {
    repository: String,
    branch: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct RawDependencies {
    ora: Option<String>,
}

/// Holds the descriptive metadata shared by both manifest forms.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RawMetadata {
    name: String,
    namespace: String,
    kind: String,
    version: String,
    description: String,
    homepage: Option<String>,
    license: Option<String>,
    head: Option<RawHead>,
    dependencies: Option<RawDependencies>,
    workbench: Option<RawWorkbench>,
    webview: Option<RawWebview>,
}

impl RawPluginManifest {
    /// Splits the release form into shared metadata and required download fields.
    fn into_parts(self) -> (RawMetadata, u64, String, String) {
        let metadata = RawMetadata {
            name: self.name,
            namespace: self.namespace,
            kind: self.kind,
            version: self.version,
            description: self.description,
            homepage: self.homepage,
            license: self.license,
            head: self.head,
            dependencies: self.dependencies,
            workbench: self.workbench,
            webview: self.webview,
        };
        (metadata, self.resolver, self.url, self.sha256)
    }
}

impl RawInstalledManifest {
    /// Splits the installed form into shared metadata and optional download fields.
    fn into_parts(self) -> (RawMetadata, Option<u64>, Option<String>, Option<String>) {
        let metadata = RawMetadata {
            name: self.name,
            namespace: self.namespace,
            kind: self.kind,
            version: self.version,
            description: self.description,
            homepage: self.homepage,
            license: self.license,
            head: self.head,
            dependencies: self.dependencies,
            workbench: self.workbench,
            webview: self.webview,
        };
        (metadata, self.resolver, self.url, self.sha256)
    }
}

#[derive(Clone, Copy)]
enum TextPolicy {
    Description,
    License,
}

impl TextPolicy {
    /// Returns the maximum byte length for this field category.
    fn max_bytes(self) -> usize {
        match self {
            Self::Description => 1000,
            Self::License => 256,
        }
    }
}

/// Applies the shared non-empty, whitespace, control, and field-specific text policies.
fn validate_text(value: &str, policy: TextPolicy) -> Result<(), InvalidFieldReason> {
    if value.is_empty() {
        return Err(InvalidFieldReason::Empty);
    }
    if value.len() > policy.max_bytes() {
        return Err(InvalidFieldReason::TooLong {
            max_bytes: policy.max_bytes(),
            actual_bytes: value.len(),
        });
    }
    if value.chars().next().is_some_and(char::is_whitespace)
        || value.chars().next_back().is_some_and(char::is_whitespace)
    {
        return Err(InvalidFieldReason::LeadingOrTrailingWhitespace);
    }
    if value.chars().any(char::is_control) {
        return Err(InvalidFieldReason::ContainsControlCharacter);
    }
    if matches!(policy, TextPolicy::License) && !value.is_ascii() {
        return Err(InvalidFieldReason::NonAscii);
    }

    Ok(())
}

/// Attaches a structured field path to one semantic validation reason.
fn invalid_field(field: ManifestField, reason: InvalidFieldReason) -> ManifestError {
    ManifestError::InvalidField { field, reason }
}
