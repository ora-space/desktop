use crate::webview::RawWebview;
use crate::workbench::RawWorkbench;
use crate::{
    HomepageUrl, HookTarget, InvalidFieldReason, ManifestError, ManifestField, PluginKind,
    PluginName, PluginNamespace, PluginWebview, PluginWorkbench, ReleaseUrl, RepositoryUrl,
    Sha256Digest,
};
use ora_utils::GitBranchName;
use semver::{Version, VersionReq};
use serde::Deserialize;
use std::str::FromStr;

const SUPPORTED_RESOLVER: u64 = 1;

/// Distinguishes the marketplace listing form from the in-package installed form so each can
/// refuse the other form's download/artifact sections.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ManifestForm {
    Release,
    Installed,
}

/// Holds one fully validated plugin release manifest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginManifest {
    pub(crate) resolver: u64,
    pub(crate) name: PluginName,
    pub(crate) title: String,
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
    pub(crate) release_source: Option<PluginReleaseSource>,
    pub(crate) artifact: Option<PluginArtifact>,
}

impl PluginManifest {
    /// Parses and validates one plugin release manifest from TOML text.
    ///
    /// A release manifest is the marketplace form. It spells the name segment `identifier` like
    /// an installed package and may carry the optional `url`/`sha256` download metadata.
    pub fn parse(source: &str) -> Result<Self, ManifestError> {
        let raw: RawPluginManifest = deserialize(source)?;
        let (metadata, resolver, url, sha256) = raw.into_parts();
        Self::from_raw_parts(metadata, resolver, url, sha256, ManifestForm::Release)
    }

    /// Parses and validates an installed plugin's manifest (the `orax.toml` shipped inside a
    /// package). Installed manifests carry descriptive metadata only; the download-only `url` and
    /// `sha256` fields are optional, and an omitted `resolver` is accepted as the current version.
    pub fn parse_installed(source: &str) -> Result<Self, ManifestError> {
        let raw: RawInstalledManifest = deserialize(source)?;
        let (metadata, resolver, url, sha256) = raw.into_parts();
        let resolver = resolver.unwrap_or(SUPPORTED_RESOLVER);
        Self::from_raw_parts(metadata, resolver, url, sha256, ManifestForm::Installed)
    }

    /// Applies every semantic validation rule to the values shared by both manifest forms,
    /// keeping the release and installed schemas on one validated domain model.
    ///
    /// Both forms spell the name segment `identifier`, so a rejected name always reports that
    /// field.
    fn from_raw_parts(
        metadata: RawMetadata,
        resolver: u64,
        url: Option<String>,
        sha256: Option<String>,
        form: ManifestForm,
    ) -> Result<Self, ManifestError> {
        if resolver != SUPPORTED_RESOLVER {
            return Err(ManifestError::UnsupportedResolver { found: resolver });
        }

        // Keep semantic conversion explicit so the first error follows schema declaration order.
        let name = PluginName::parse(&metadata.name)
            .map_err(|reason| invalid_field(ManifestField::Identifier, reason.into()))?;
        // The display title is descriptive metadata; a manifest that omits it falls back to the
        // identifier so a plugin never lacks a name to show.
        let title = match metadata.title.as_deref() {
            Some(value) => {
                validate_text(value, TextPolicy::Title)
                    .map_err(|reason| invalid_field(ManifestField::Title, reason))?;
                value.to_owned()
            }
            None => name.as_str().to_owned(),
        };
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
        let release_source = validate_release_source(
            url.as_ref(),
            sha256.as_ref(),
            metadata.targets.as_deref(),
            kind,
            form,
        )?;
        if metadata.artifact.is_some() && matches!(form, ManifestForm::Release) {
            return Err(invalid_field(
                ManifestField::Artifact,
                InvalidFieldReason::ArtifactNotAllowedOnRelease,
            ));
        }
        if metadata.artifact.is_some() && !matches!(kind, PluginKind::Hook) {
            return Err(invalid_field(
                ManifestField::Artifact,
                InvalidFieldReason::NotAllowedForKind { kind },
            ));
        }
        // A targeted Hook package self-declares its host triple in `[artifact]`; an installed
        // Hook without that section cannot prove host compatibility independently of marketplace
        // metadata.
        if matches!(form, ManifestForm::Installed)
            && matches!(kind, PluginKind::Hook)
            && metadata.artifact.is_none()
        {
            return Err(invalid_field(
                ManifestField::Artifact,
                InvalidFieldReason::MissingForKind { kind },
            ));
        }
        let artifact = metadata
            .artifact
            .map(PluginArtifact::try_from)
            .transpose()?;
        let (workbench, webview) =
            validate_kind_sections(kind, metadata.workbench, metadata.webview)?;

        Ok(Self {
            resolver,
            name,
            title,
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
            release_source,
            artifact,
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

    /// Returns the human-readable display title, falling back to the identifier when unset.
    pub fn title(&self) -> &str {
        &self.title
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

    /// Returns the release source selection: one universal URL/digest pair, or one or more
    /// unique exact-target artifacts. `None` means a manifest that declares neither form.
    pub fn release_source(&self) -> Option<&PluginReleaseSource> {
        self.release_source.as_ref()
    }

    /// Returns the installed artifact self-declaration carried inside a targeted package.
    ///
    /// A universal release carries no artifact target; a targeted archive declares exactly one so
    /// local import and online install apply the same host-compatibility boundary.
    pub fn artifact(&self) -> Option<&PluginArtifact> {
        self.artifact.as_ref()
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
        (
            PluginKind::Agent
            | PluginKind::Webview
            | PluginKind::Skill
            | PluginKind::Mcp
            | PluginKind::Hook,
            Some(_),
        ) => {
            return Err(invalid_field(
                ManifestField::Workbench,
                InvalidFieldReason::NotAllowedForKind { kind },
            ));
        }
        (
            PluginKind::Agent
            | PluginKind::Webview
            | PluginKind::Skill
            | PluginKind::Mcp
            | PluginKind::Hook,
            None,
        ) => None,
    };
    let webview = match (kind, webview) {
        (PluginKind::Webview, Some(webview)) => Some(PluginWebview::try_from(webview)?),
        (PluginKind::Webview, None) => {
            return Err(invalid_field(
                ManifestField::Webview,
                InvalidFieldReason::MissingForKind { kind },
            ));
        }
        (
            PluginKind::Agent
            | PluginKind::Workbench
            | PluginKind::Skill
            | PluginKind::Mcp
            | PluginKind::Hook,
            Some(_),
        ) => {
            return Err(invalid_field(
                ManifestField::Webview,
                InvalidFieldReason::NotAllowedForKind { kind },
            ));
        }
        (
            PluginKind::Agent
            | PluginKind::Workbench
            | PluginKind::Skill
            | PluginKind::Mcp
            | PluginKind::Hook,
            None,
        ) => None,
    };

    Ok((workbench, webview))
}

/// Models the mutually exclusive resolver-one release source.
///
/// A release is either one universal URL/digest pair installable on every host target, or one or
/// more unique exact-target artifacts. Modeling the two as one enum keeps URL-selection
/// precedence unambiguous: a manifest can never carry both forms.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginReleaseSource {
    /// One URL/digest pair installable on every host target the plugin's Ora version supports.
    Universal {
        url: ReleaseUrl,
        sha256: Sha256Digest,
    },
    /// One or more exact-target artifacts, each carrying its own URL and digest.
    Targets(Vec<PluginReleaseTarget>),
}

/// Holds one validated target-specific release artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginReleaseTarget {
    pub(crate) target: HookTarget,
    pub(crate) url: ReleaseUrl,
    pub(crate) sha256: Sha256Digest,
}

impl PluginReleaseTarget {
    /// Returns the target triple this artifact is built for.
    pub fn target(&self) -> &HookTarget {
        &self.target
    }

    /// Returns the download URL of this target artifact.
    pub fn url(&self) -> &ReleaseUrl {
        &self.url
    }

    /// Returns the SHA-256 digest of this target artifact.
    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }
}

/// Holds the installed artifact self-declaration carried inside a targeted package.
///
/// The installed form carries no download URL, digest, or target list; instead it repeats one
/// target triple so the host can verify the physical artifact matches the host it runs on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginArtifact {
    pub(crate) target: HookTarget,
}

impl PluginArtifact {
    /// Returns the target triple the installed physical artifact self-declares.
    pub fn target(&self) -> &HookTarget {
        &self.target
    }
}

impl TryFrom<RawArtifact> for PluginArtifact {
    type Error = ManifestError;

    /// Converts the raw artifact section after validating its single target field.
    fn try_from(raw: RawArtifact) -> Result<Self, Self::Error> {
        let target = HookTarget::parse(&raw.target)
            .map_err(|reason| invalid_field(ManifestField::ArtifactTarget, reason.into()))?;
        Ok(Self { target })
    }
}

/// Compiles the mutually exclusive release source from the optional universal and targeted
/// declarations, enforcing that a manifest declares exactly one form.
///
/// The `url`/`sha256` top-level fields describe a universal artifact; the `[[targets]]` array
/// describes one or more exact-target artifacts. The two forms are mutually exclusive so a
/// release can never carry ambiguous download precedence. The universal form is available to
/// every kind that may declare a release; the targeted form is exclusive to `hook` because only
/// a Hook Plugin ships target-specific native binaries whose host-compatibility the host must
/// check before download.
fn validate_release_source(
    url: Option<&ReleaseUrl>,
    sha256: Option<&Sha256Digest>,
    targets: Option<&[RawReleaseTarget]>,
    kind: PluginKind,
    form: ManifestForm,
) -> Result<Option<PluginReleaseSource>, ManifestError> {
    let has_universal = url.is_some() || sha256.is_some();
    let has_targets = targets.is_some_and(|entries| !entries.is_empty());

    // Installed packages may carry a self-declared `sha256` for the archive they came from, but
    // they never advertise a downloadable release: `[[targets]]` would smuggle download URLs into
    // the installed form, and `url`/`sha256` are not a marketplace source.
    if matches!(form, ManifestForm::Installed) {
        if has_targets {
            return Err(invalid_field(
                ManifestField::Targets,
                InvalidFieldReason::TargetsNotAllowedOnInstalled,
            ));
        }
        return Ok(None);
    }

    // Neither form means the manifest declares no downloadable release at all; marketplace listings
    // without a release are still indexed, they simply cannot be installed online.
    if !has_universal && !has_targets {
        return Ok(None);
    }
    if has_universal && has_targets {
        return Err(invalid_field(
            ManifestField::Targets,
            InvalidFieldReason::DuplicateReleaseSource,
        ));
    }
    if has_universal {
        // A universal artifact must carry both halves: a URL without a digest cannot be verified
        // and a digest without a URL cannot be downloaded.
        let Some(url) = url else {
            return Err(invalid_field(
                ManifestField::Url,
                InvalidFieldReason::MissingUniversalReleaseField,
            ));
        };
        let Some(sha256) = sha256 else {
            return Err(invalid_field(
                ManifestField::Sha256,
                InvalidFieldReason::MissingUniversalReleaseField,
            ));
        };
        return Ok(Some(PluginReleaseSource::Universal {
            url: url.clone(),
            sha256: *sha256,
        }));
    }

    // The targeted form is exclusive to Hook Plugins: only a Hook ships native per-target binaries
    // whose host compatibility the marketplace must advertise before download.
    if !matches!(kind, PluginKind::Hook) {
        return Err(invalid_field(
            ManifestField::Targets,
            InvalidFieldReason::NotAllowedForKind { kind },
        ));
    }
    let Some(raw_targets) = targets else {
        // `has_targets` already guaranteed the array is present and non-empty; reaching here with
        // `None` is unreachable, but the guard keeps the function total without `expect`.
        return Ok(None);
    };
    let mut compiled: Vec<PluginReleaseTarget> = Vec::with_capacity(raw_targets.len());
    let mut seen: Vec<HookTarget> = Vec::with_capacity(raw_targets.len());
    for (index, raw) in raw_targets.iter().enumerate() {
        let target = HookTarget::parse(&raw.target).map_err(|reason| {
            invalid_field(ManifestField::ReleaseTargetTarget { index }, reason.into())
        })?;
        if seen.iter().any(|seen| seen == &target) {
            return Err(invalid_field(
                ManifestField::ReleaseTargetTarget { index },
                InvalidFieldReason::Duplicate,
            ));
        }
        let url = ReleaseUrl::parse(&raw.url).map_err(|reason| {
            invalid_field(ManifestField::ReleaseTargetUrl { index }, reason.into())
        })?;
        let sha256 = Sha256Digest::parse(&raw.sha256).map_err(|reason| {
            invalid_field(ManifestField::ReleaseTargetSha256 { index }, reason.into())
        })?;
        seen.push(target.clone());
        compiled.push(PluginReleaseTarget {
            target,
            url,
            sha256,
        });
    }
    Ok(Some(PluginReleaseSource::Targets(compiled)))
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
    identifier: String,
    title: Option<String>,
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
    #[serde(default)]
    targets: Option<Vec<RawReleaseTarget>>,
    #[serde(default)]
    artifact: Option<RawArtifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawInstalledManifest {
    resolver: Option<u64>,
    /// Identifier segment of the installed package, spelled `identifier` (not `name`) because an
    /// installed manifest is only ever addressed by the full id the host resolves from its name
    /// and namespace.
    identifier: String,
    title: Option<String>,
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
    #[serde(default)]
    targets: Option<Vec<RawReleaseTarget>>,
    #[serde(default)]
    artifact: Option<RawArtifact>,
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

/// Raw form of one `[[targets]]` release entry.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct RawReleaseTarget {
    target: String,
    url: String,
    sha256: String,
}

/// Raw form of the installed `[artifact]` self-declaration.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct RawArtifact {
    target: String,
}

/// Holds the descriptive metadata shared by both manifest forms.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RawMetadata {
    name: String,
    title: Option<String>,
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
    targets: Option<Vec<RawReleaseTarget>>,
    artifact: Option<RawArtifact>,
}

impl RawPluginManifest {
    /// Splits the release form into shared metadata and optional download fields.
    fn into_parts(self) -> (RawMetadata, u64, Option<String>, Option<String>) {
        let metadata = RawMetadata {
            // The marketplace release form spells the name segment `identifier` like the
            // installed form, and the download fields are optional now that the marketplace no
            // longer publishes `.orax` release URLs.
            name: self.identifier,
            title: self.title,
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
            targets: self.targets,
            artifact: self.artifact,
        };
        (metadata, self.resolver, self.url, self.sha256)
    }
}

impl RawInstalledManifest {
    /// Splits the installed form into shared metadata and optional download fields.
    fn into_parts(self) -> (RawMetadata, Option<u64>, Option<String>, Option<String>) {
        let metadata = RawMetadata {
            // The installed manifest spells the name segment `identifier`, mapping it onto the
            // shared metadata name so both forms converge on one validated domain model.
            name: self.identifier,
            title: self.title,
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
            targets: self.targets,
            artifact: self.artifact,
        };
        (metadata, self.resolver, self.url, self.sha256)
    }
}

#[derive(Clone, Copy)]
enum TextPolicy {
    Title,
    Description,
    License,
}

impl TextPolicy {
    /// Returns the maximum byte length for this field category.
    fn max_bytes(self) -> usize {
        match self {
            Self::Title => 128,
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
