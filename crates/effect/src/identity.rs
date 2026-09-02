use ora_domain::{WorkspaceId, validate_skill_name};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest as _, Sha256};
use std::fmt::{self, Display, Formatter};
use std::hash::{Hash, Hasher};
use thiserror::Error;
use uuid::Uuid;

/// Identifies the only Effect isolation shape supported by the first system version.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "identity", rename_all = "snake_case")]
pub enum EffectScopeId {
    Workspace(WorkspaceId),
}

impl EffectScopeId {
    /// Produces the tagged key used to prevent identities from colliding across future scope kinds.
    pub fn storage_key(&self) -> String {
        match self {
            Self::Workspace(workspace_id) => format!("workspace:{workspace_id}"),
        }
    }

    /// Returns the Workspace identity carried by the first-version scope.
    pub fn workspace_id(&self) -> &WorkspaceId {
        match self {
            Self::Workspace(workspace_id) => workspace_id,
        }
    }
}

impl Display for EffectScopeId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.storage_key())
    }
}

/// Names one versioned Effect definition family without coupling it to a Consumer kind.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct EffectKind(String);

impl EffectKind {
    /// Builds the built-in, namespaced Skill definition identity.
    pub fn skill() -> Self {
        Self("ora/skill".to_string())
    }

    /// Validates a stable namespaced identity before it enters planning or persistence.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        validate_namespaced(&value)
            .map_err(|()| IdentityError::InvalidEffectKind(value.clone()))?;
        Ok(Self(value))
    }

    /// Returns the stable namespaced representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for EffectKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Names one Consumer family while leaving its stable identity in a separate value.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ConsumerKind(String);

impl ConsumerKind {
    /// Builds the built-in Agent plugin Consumer kind.
    pub fn agent_plugin() -> Self {
        Self("ora/agent-plugin".to_string())
    }

    /// Validates a stable namespaced Consumer kind.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        validate_namespaced(&value)
            .map_err(|()| IdentityError::InvalidConsumerKind(value.clone()))?;
        Ok(Self(value))
    }

    /// Returns the stable namespaced representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ConsumerKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Stable identity of a runtime that consumes Effect results.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ConsumerIdentity {
    pub kind: ConsumerKind,
    pub stable_key: String,
}

impl ConsumerIdentity {
    /// Refuses display names or empty connection-local values as stable Consumer identity.
    pub fn new(kind: ConsumerKind, stable_key: impl Into<String>) -> Result<Self, IdentityError> {
        let stable_key = stable_key.into();
        if stable_key.trim().is_empty() {
            return Err(IdentityError::EmptyConsumerIdentity);
        }
        Ok(Self { kind, stable_key })
    }

    /// Derives an opaque database key while preserving the typed parts on the Consumer row.
    pub fn storage_key(&self) -> String {
        let mut digest = Sha256::new();
        digest.update(b"ora-effect-consumer-v1\0");
        digest.update(self.kind.as_str().as_bytes());
        digest.update([0]);
        digest.update(self.stable_key.as_bytes());
        format!("consumer:{:x}", digest.finalize())
    }
}

impl Display for ConsumerIdentity {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.kind, self.stable_key)
    }
}

/// A monotonically increasing complete Desired State version for one Effect Scope.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize,
)]
pub struct Generation(u64);

impl Generation {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the storage and contract representation.
    pub fn value(self) -> u64 {
        self.0
    }

    /// Advances the complete declaration version without wrapping into an older generation.
    pub fn next(self) -> Result<Self, IdentityError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(IdentityError::GenerationExhausted)
    }
}

/// Monotonic optimistic version of one Target or Resource status snapshot.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct StatusVersion(u64);

impl StatusVersion {
    pub fn initial() -> Self {
        Self(1)
    }

    pub fn new(value: u64) -> Result<Self, IdentityError> {
        if value == 0 {
            return Err(IdentityError::ZeroStatusVersion);
        }
        Ok(Self(value))
    }

    /// Returns the persistence representation.
    pub fn value(self) -> u64 {
        self.0
    }

    /// Advances a status after one atomic transition.
    pub fn next(self) -> Result<Self, IdentityError> {
        self.0
            .checked_add(1)
            .map(Self)
            .ok_or(IdentityError::StatusVersionExhausted)
    }
}

/// Monotonic token that fences stale Target and Resource workers.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct FencingToken(u64);

impl FencingToken {
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the persistence representation.
    pub fn value(self) -> u64 {
        self.0
    }
}

/// Counts retry attempts without being confused with a generation or fencing token.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RetryAttempt(u32);

impl RetryAttempt {
    pub fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the persistence representation.
    pub fn value(self) -> u32 {
        self.0
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

            /// Creates a random identity whose authority cannot be inferred from external state.
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
    EffectSourceIdentity,
    "Identifies a stable Effect source across publications."
);
opaque_id!(EffectRevisionId, "Addresses one immutable source revision.");
opaque_id!(
    DesiredEffectIdentity,
    "Identifies one stable item of Desired Effect intent."
);
opaque_id!(
    ConsumerRevisionId,
    "Addresses one immutable Consumer capability snapshot."
);
opaque_id!(
    EffectTargetId,
    "Identifies one Consumer convergence instance in one Scope."
);
opaque_id!(
    EffectResourceId,
    "Identifies one independently mutable external Resource."
);
opaque_id!(
    ManagedIdentity,
    "Proves one continuous external ownership lifecycle."
);
opaque_id!(
    ConditionId,
    "Identifies one current structured convergence fact."
);
opaque_id!(
    ReconcileAttemptId,
    "Identifies one immutable Target reconcile orchestration."
);
opaque_id!(
    EffectOperationId,
    "Identifies one immutable Resource mutation journal."
);
opaque_id!(
    ArtifactId,
    "Identifies one operation-owned temporary external artifact."
);
opaque_id!(
    AuditEventId,
    "Identifies one append-only Effect business-history entry."
);

impl ManagedIdentity {
    /// Allocates a repeatable identity for one Resource/Desired ownership slot before a ledger exists.
    pub fn for_intent(resource: &EffectResourceId, desired_effect: &DesiredEffectIdentity) -> Self {
        let mut digest = Sha256::new();
        digest.update(b"ora-effect-managed-intent-v1\0");
        digest.update(resource.as_str().as_bytes());
        digest.update([0]);
        digest.update(desired_effect.as_str().as_bytes());
        Self(format!("managed:{:x}", digest.finalize()))
    }
}

macro_rules! named_identity {
    ($name:ident, $doc:literal, $error:ident) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        pub struct $name(String);

        impl $name {
            /// Rejects an empty identity because it cannot select a stable adapter contract.
            pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
                let value = value.into();
                if value.trim().is_empty() {
                    return Err(IdentityError::$error);
                }
                Ok(Self(value))
            }

            /// Returns the stable persistence representation.
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

named_identity!(
    ConsumerAdapterIdentity,
    "Selects a Consumer protocol adapter.",
    EmptyConsumerAdapterIdentity
);
named_identity!(
    ResourceAdapterIdentity,
    "Selects a Resource mutation adapter.",
    EmptyResourceAdapterIdentity
);
named_identity!(
    ResourceKey,
    "Names one adapter-normalized physical Resource within a Scope.",
    EmptyResourceKey
);
named_identity!(
    NativeResourceIdentity,
    "Names one item in a Resource's native identity system.",
    EmptyNativeResourceIdentity
);
named_identity!(
    SourceRevisionKey,
    "Names one exact immutable revision in its source system.",
    EmptySourceRevisionKey
);
named_identity!(
    WorkerIdentity,
    "Identifies the worker holding a durable claim.",
    EmptyWorkerIdentity
);

impl ResourceKey {
    /// Constructs a key whose non-empty shape was already proven by a typed normalizer.
    pub(crate) fn from_normalized(value: String) -> Self {
        debug_assert!(!value.is_empty());
        Self(value)
    }
}

/// A SHA-256 content identity for immutable input or a normalized projection.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Digest(String);

impl Digest {
    /// Computes the algorithm-prefixed digest used for immutable content identity.
    pub fn sha256(bytes: &[u8]) -> Self {
        Self(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    /// Validates a persisted digest before allowing it into domain state.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        validate_hash(value.into(), IdentityError::InvalidDigest).map(Self)
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

/// A SHA-256 proof of observed external Resource state, never ownership.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct Fingerprint(String);

impl Fingerprint {
    /// Computes an algorithm-prefixed fingerprint over normalized observed bytes.
    pub fn sha256(bytes: &[u8]) -> Self {
        Self(format!("sha256:{:x}", Sha256::digest(bytes)))
    }

    /// Validates a persisted external-state fingerprint.
    pub fn parse(value: impl Into<String>) -> Result<Self, IdentityError> {
        validate_hash(value.into(), IdentityError::InvalidFingerprint).map(Self)
    }

    /// Returns the persistence-safe fingerprint representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for Fingerprint {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl From<ora_utils::directory::DirectoryFingerprint> for Fingerprint {
    /// Converts a validated generic directory hash without reparsing the same representation.
    fn from(value: ora_utils::directory::DirectoryFingerprint) -> Self {
        Self(value.to_string())
    }
}

/// Digest of a complete deterministic Target or Resource projection.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ProjectionDigest(Digest);

impl ProjectionDigest {
    pub fn new(digest: Digest) -> Self {
        Self(digest)
    }

    /// Returns the underlying immutable-content digest.
    pub fn digest(&self) -> &Digest {
        &self.0
    }
}

impl Display for ProjectionDigest {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        Display::fmt(&self.0, formatter)
    }
}

/// A globally case-insensitive Skill identity that retains display spelling.
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

    /// Returns the spelling used by Resource adapters and user interfaces.
    pub fn as_str(&self) -> &str {
        &self.display
    }

    /// Returns the normalized identity used for collision detection.
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

/// Reports invalid construction of strongly typed Effect identities.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum IdentityError {
    #[error("invalid Effect kind: {0}")]
    InvalidEffectKind(String),
    #[error("invalid Consumer kind: {0}")]
    InvalidConsumerKind(String),
    #[error("Consumer identity must not be empty")]
    EmptyConsumerIdentity,
    #[error("Consumer adapter identity must not be empty")]
    EmptyConsumerAdapterIdentity,
    #[error("Resource adapter identity must not be empty")]
    EmptyResourceAdapterIdentity,
    #[error("Resource key must not be empty")]
    EmptyResourceKey,
    #[error("native Resource identity must not be empty")]
    EmptyNativeResourceIdentity,
    #[error("source revision key must not be empty")]
    EmptySourceRevisionKey,
    #[error("worker identity must not be empty")]
    EmptyWorkerIdentity,
    #[error("invalid digest: {0}")]
    InvalidDigest(String),
    #[error("invalid fingerprint: {0}")]
    InvalidFingerprint(String),
    #[error("invalid Skill name: {0}")]
    InvalidSkillName(String),
    #[error("Effect generation is exhausted")]
    GenerationExhausted,
    #[error("status version must be greater than zero")]
    ZeroStatusVersion,
    #[error("status version is exhausted")]
    StatusVersionExhausted,
}

/// Requires a namespace and local name so kind strings remain globally stable.
fn validate_namespaced(value: &str) -> Result<(), ()> {
    let Some((namespace, name)) = value.split_once('/') else {
        return Err(());
    };
    if namespace.is_empty() || name.is_empty() || name.contains('/') || value.trim() != value {
        return Err(());
    }
    Ok(())
}

/// Shares the wire-format validation while retaining distinct Digest and Fingerprint types.
fn validate_hash(
    value: String,
    error: impl FnOnce(String) -> IdentityError,
) -> Result<String, IdentityError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(error(value));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(error(value));
    }
    Ok(format!("sha256:{}", hex.to_ascii_lowercase()))
}
