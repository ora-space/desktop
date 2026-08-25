use ora_domain::{Namespace, WorkspaceId, validate_skill_name};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest as _, Sha256};
use std::fmt::{self, Display, Formatter};
use std::hash::{Hash, Hasher};
use thiserror::Error;
use uuid::Uuid;

/// Identifies which upstream catalog owns a selected Skill.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Local,
    Plugin,
}

/// A globally case-insensitive Skill identity that retains its display spelling.
#[derive(Clone, Debug)]
pub struct SkillName {
    display: String,
    canonical: String,
}

impl SkillName {
    /// Validates one display name and stores its case-insensitive identity alongside it.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        let display = value.into();
        validate_skill_name(&display)
            .map_err(|_| IdentityError::InvalidSkillName(display.clone()))?;
        let canonical = display.to_ascii_lowercase();
        Ok(Self { display, canonical })
    }

    /// Returns the spelling that should be used for the target directory and UI.
    pub fn as_str(&self) -> &str {
        &self.display
    }

    /// Returns the normalized identity used by persistence and collision detection.
    pub fn canonical(&self) -> &str {
        &self.canonical
    }
}

impl PartialEq for SkillName {
    fn eq(&self, other: &Self) -> bool {
        self.canonical == other.canonical
    }
}

impl Eq for SkillName {}

impl PartialOrd for SkillName {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SkillName {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.canonical.cmp(&other.canonical)
    }
}

impl Hash for SkillName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.canonical.hash(state);
    }
}

impl Display for SkillName {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.display)
    }
}

impl Serialize for SkillName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.display)
    }
}

impl<'de> Deserialize<'de> for SkillName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

/// An opaque source revision compared only by exact equality.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SourceVersion(String);

impl SourceVersion {
    /// Rejects empty revisions because they cannot identify an upstream state.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdentityError::EmptySourceVersion);
        }
        Ok(Self(value))
    }

    /// Returns the exact upstream representation without interpreting it.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for SourceVersion {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A SHA-256 digest over the unmodified root `SKILL.md` bytes.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Digest(String);

impl Digest {
    /// Computes the algorithm-prefixed digest used in exact Skill state identity.
    pub fn sha256(bytes: &[u8]) -> Self {
        Self(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    /// Validates a persisted digest before allowing it into domain state.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(IdentityError::InvalidDigest(value));
        };
        if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(IdentityError::InvalidDigest(value));
        }
        Ok(Self(format!("sha256:{}", hex.to_ascii_lowercase())))
    }

    /// Returns the persistence-safe digest representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for Digest {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Identifies the stable upstream choice independently of its current revision.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SkillSelectionKey {
    pub source_kind: SourceKind,
    pub namespace: Namespace,
    pub name: SkillName,
}

impl SkillSelectionKey {
    pub fn new(source_kind: SourceKind, namespace: Namespace, name: SkillName) -> Self {
        Self {
            source_kind,
            namespace,
            name,
        }
    }
}

/// A monotonically increasing complete Workspace specification revision.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub struct Generation(u64);

impl Generation {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the storage representation.
    pub fn value(self) -> u64 {
        self.0
    }

    /// Advances a generation without wrapping into an older state.
    pub fn next(self) -> Result<Self, IdentityError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(IdentityError::GenerationExhausted)
    }
}

macro_rules! opaque_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Creates a random identity that cannot be inferred from a target locator.
            pub fn random() -> Self {
                Self(Uuid::new_v4().to_string())
            }

            /// Returns the opaque persistence representation.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Display for $name {
            fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

opaque_id!(
    ManagedIdentity,
    "Proves one continuous database ownership lifecycle."
);
opaque_id!(
    EffectOperationId,
    "Identifies one durable filesystem transaction."
);

/// Identifies a physical Workspace surface by normalized relative path.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SurfaceKey(String);

impl SurfaceKey {
    /// Derives a stable key without exposing it as ownership evidence.
    pub fn for_workspace(workspace_id: &WorkspaceId, normalized_relative_path: &str) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"ora-skill-surface-v1\0");
        digest.update(workspace_id.as_ref().as_bytes());
        digest.update([0]);
        digest.update(normalized_relative_path.as_bytes());
        Self(format!("surface:{:x}", digest.finalize()))
    }

    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the stable persistence representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for SurfaceKey {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Identifies one consumer of a physical surface.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ConsumerId(String);

impl ConsumerId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the plugin identity used by status subjects.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ConsumerId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A persisted full-directory fingerprint separate from source manifest digest.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct AppliedFingerprint(String);

impl AppliedFingerprint {
    /// Accepts only the shared SHA-256 fingerprint representation.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(IdentityError::InvalidFingerprint(value));
        };
        if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(IdentityError::InvalidFingerprint(value));
        }
        Ok(Self(format!("sha256:{}", hex.to_ascii_lowercase())))
    }

    /// Returns the fingerprint representation used by marker validation and recovery.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for AppliedFingerprint {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Reports invalid construction of strongly typed Effect identity values.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum IdentityError {
    #[error("invalid skill name: {0}")]
    InvalidSkillName(String),
    #[error("source version must not be empty")]
    EmptySourceVersion,
    #[error("invalid skill digest: {0}")]
    InvalidDigest(String),
    #[error("invalid applied fingerprint: {0}")]
    InvalidFingerprint(String),
    #[error("workspace effect generation is exhausted")]
    GenerationExhausted,
}
