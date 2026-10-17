use crate::{InvalidFieldReason, ManifestError, ManifestField, PluginKind, Sha256Digest};

/// Maximum accepted byte length of the declared static-copy path.
pub const MAX_COPY_PATH_BYTES: usize = 256;

/// Maximum accepted byte length of one segment inside the copy path.
const MAX_COPY_SEGMENT_BYTES: usize = 128;

/// Holds the release-form `[agent_configuration]` publication association of one Agent listing.
///
/// The section binds the marketplace's static declaration copy to the exact release packages the
/// listing names: the declaration digest identifies the `assets/config.json` bytes every package
/// of this release must carry, and `copy` locates the byte-identical static copy inside the
/// registry entry the marketplace sync reads. The namespace half of the canonical plugin id never
/// appears here because it belongs to the publishing marketplace source, not the manifest.
///
/// The declaration *version* is carried as data on purpose: the manifest resolver, the declaration
/// schema, and the adapter contract are three separate version domains, so this crate records the
/// association without deciding whether its host can compile that declaration version. Consumers
/// with the declaration compiler at hand (marketplace sync, package installation) reject versions
/// they do not support instead of treating the section as absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginAgentConfiguration {
    schema_version: u32,
    declaration_sha256: Sha256Digest,
    copy: String,
}

impl PluginAgentConfiguration {
    /// Returns the declaration schema version the associated copy declares.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the SHA-256 digest of the exact declaration bytes this release binds.
    pub fn declaration_sha256(&self) -> &Sha256Digest {
        &self.declaration_sha256
    }

    /// Returns the registry-entry-relative path of the static declaration copy.
    pub fn copy(&self) -> &str {
        &self.copy
    }
}

impl TryFrom<RawAgentConfiguration> for PluginAgentConfiguration {
    type Error = ManifestError;

    /// Converts the raw section after validating every field.
    fn try_from(raw: RawAgentConfiguration) -> Result<Self, Self::Error> {
        if raw.schema_version == 0 {
            return Err(invalid_field(
                ManifestField::AgentConfigurationSchemaVersion,
                InvalidFieldReason::InvalidSchemaVersion,
            ));
        }
        let declaration_sha256 =
            Sha256Digest::parse(&raw.declaration_sha256).map_err(|reason| {
                invalid_field(
                    ManifestField::AgentConfigurationDeclarationSha256,
                    reason.into(),
                )
            })?;
        let copy = validate_copy_path(&raw.copy)?;
        Ok(Self {
            schema_version: raw.schema_version,
            declaration_sha256,
            copy,
        })
    }
}

/// Applies the release-form policy for the `[agent_configuration]` section.
///
/// The section exists only on marketplace listings: an installed package repeats no download
/// locators, and the in-package `assets/config.json` is itself the declaration of record, so a
/// copy path or digest on the installed form could only disagree with the bytes beside it. Only
/// an `agent` listing may carry it, and only when the listing declares a release source, because
/// the association's whole meaning is binding declaration bytes to exact package digests.
pub(crate) fn validate_agent_configuration(
    raw: Option<RawAgentConfiguration>,
    kind: PluginKind,
    installed_form: bool,
    has_release_source: bool,
) -> Result<Option<PluginAgentConfiguration>, ManifestError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if installed_form {
        return Err(invalid_field(
            ManifestField::AgentConfiguration,
            InvalidFieldReason::AgentConfigurationNotAllowedOnInstalled,
        ));
    }
    if !matches!(kind, PluginKind::Agent) {
        return Err(invalid_field(
            ManifestField::AgentConfiguration,
            InvalidFieldReason::NotAllowedForKind { kind },
        ));
    }
    if !has_release_source {
        return Err(invalid_field(
            ManifestField::AgentConfiguration,
            InvalidFieldReason::DeclarationRequiresRelease,
        ));
    }
    raw.try_into().map(Some)
}

/// Validates one registry-entry-relative copy path against the closed path grammar.
///
/// Segments must match `[A-Za-z0-9][A-Za-z0-9._-]*`: the grammar excludes separators and escapes
/// (`/` inside a segment, `\`, `:`), whitespace, control characters, and non-ASCII, so a validated
/// path can never name an absolute location, a parent directory, a Windows drive, or a UNC share
/// on any host. The marketplace sync still resolves the path inside its checkout; this grammar is
/// what makes that resolution containable without platform-specific normalization.
fn validate_copy_path(value: &str) -> Result<String, ManifestError> {
    if value.is_empty() {
        return Err(invalid_field(
            ManifestField::AgentConfigurationCopy,
            InvalidFieldReason::Empty,
        ));
    }
    if value.len() > MAX_COPY_PATH_BYTES {
        return Err(invalid_field(
            ManifestField::AgentConfigurationCopy,
            InvalidFieldReason::TooLong {
                max_bytes: MAX_COPY_PATH_BYTES,
                actual_bytes: value.len(),
            },
        ));
    }
    for segment in value.split('/') {
        let valid = !segment.is_empty()
            && segment.len() <= MAX_COPY_SEGMENT_BYTES
            && segment
                .bytes()
                .next()
                .is_some_and(|first| first.is_ascii_alphanumeric())
            && segment
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            && segment != "."
            && segment != "..";
        if !valid {
            return Err(invalid_field(
                ManifestField::AgentConfigurationCopy,
                InvalidFieldReason::InvalidCopyPath,
            ));
        }
    }
    Ok(value.to_owned())
}

/// Builds one semantic error with a stable field path, next to the schema that declares it.
fn invalid_field(field: ManifestField, reason: InvalidFieldReason) -> ManifestError {
    ManifestError::InvalidField { field, reason }
}

/// Raw form of the release-form `[agent_configuration]` section. Unknown keys are rejected: the
/// association's meaning is load-bearing for install verification, so a listing carrying fields
/// this resolver does not know fails loudly instead of being reinterpreted.
#[derive(serde::Deserialize, Clone, Debug, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RawAgentConfiguration {
    pub(crate) schema_version: u32,
    pub(crate) declaration_sha256: String,
    pub(crate) copy: String,
}
