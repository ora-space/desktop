use crate::{
    HomepageUrl, InvalidFieldReason, ManifestError, ManifestField, PluginKind, PluginName,
    PluginNamespace, ReleaseUrl, RepositoryUrl, Sha256Digest,
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
    pub(crate) url: ReleaseUrl,
    pub(crate) sha256: Sha256Digest,
    pub(crate) head: Option<PluginHead>,
    pub(crate) dependencies: Option<PluginDependencies>,
}

impl PluginManifest {
    /// Parses and validates one plugin release manifest from TOML text.
    pub fn parse(source: &str) -> Result<Self, ManifestError> {
        let raw: RawPluginManifest = toml::from_str(source).map_err(|source| {
            let span = source.span();
            ManifestError::InvalidToml { source, span }
        })?;

        if raw.resolver != SUPPORTED_RESOLVER {
            return Err(ManifestError::UnsupportedResolver {
                found: raw.resolver,
            });
        }

        // Keep semantic conversion explicit so the first error follows schema declaration order.
        let name = PluginName::parse(&raw.name)
            .map_err(|reason| invalid_field(ManifestField::Name, reason.into()))?;
        let namespace = PluginNamespace::from_str(&raw.namespace)
            .map_err(|reason| invalid_field(ManifestField::Namespace, reason.into()))?;
        let kind = PluginKind::from_str(&raw.kind)
            .map_err(|reason| invalid_field(ManifestField::Kind, reason.into()))?;
        let version = Version::parse(&raw.version).map_err(|reason| {
            invalid_field(
                ManifestField::Version,
                InvalidFieldReason::InvalidVersion(reason),
            )
        })?;
        validate_text(&raw.description, TextPolicy::Description)
            .map_err(|reason| invalid_field(ManifestField::Description, reason))?;
        let homepage = raw
            .homepage
            .as_deref()
            .map(HomepageUrl::parse)
            .transpose()
            .map_err(|reason| invalid_field(ManifestField::Homepage, reason.into()))?;
        if let Some(license) = raw.license.as_deref() {
            validate_text(license, TextPolicy::License)
                .map_err(|reason| invalid_field(ManifestField::License, reason))?;
        }
        let url = ReleaseUrl::parse(&raw.url)
            .map_err(|reason| invalid_field(ManifestField::Url, reason.into()))?;
        let sha256 = Sha256Digest::parse(&raw.sha256)
            .map_err(|reason| invalid_field(ManifestField::Sha256, reason.into()))?;

        let head = raw.head.map(PluginHead::try_from).transpose()?;
        let dependencies = raw
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

        Ok(Self {
            resolver: raw.resolver,
            name,
            namespace,
            kind,
            version,
            description: raw.description,
            homepage,
            license: raw.license,
            url,
            sha256,
            head,
            dependencies,
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

    /// Returns the release package URL.
    pub fn url(&self) -> &ReleaseUrl {
        &self.url
    }

    /// Returns the expected release package SHA-256 digest.
    pub fn sha256(&self) -> &Sha256Digest {
        &self.sha256
    }

    /// Returns optional source repository metadata.
    pub fn head(&self) -> Option<&PluginHead> {
        self.head.as_ref()
    }

    /// Returns the optional declared host dependency.
    pub fn dependencies(&self) -> Option<&PluginDependencies> {
        self.dependencies.as_ref()
    }
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
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawHead {
    repository: String,
    branch: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDependencies {
    ora: Option<String>,
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
