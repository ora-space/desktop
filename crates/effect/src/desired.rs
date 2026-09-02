use crate::{
    ConsumerIdentity, DesiredEffectIdentity, Digest, EffectKind, EffectRevisionId, EffectScopeId,
    EffectSourceIdentity, Fingerprint, Generation, SkillName, SourceRevisionKey,
};
use ora_domain::Namespace;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use thiserror::Error;

/// Identifies which catalog family owns a Skill source.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillSourceKind {
    Local,
    Plugin,
}

/// Stable, human-meaningful identity of one Skill source across immutable revisions.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct SkillSourceKey {
    pub source_kind: SkillSourceKind,
    pub namespace: Namespace,
    pub name: SkillName,
}

/// Lifecycle of a source independently from the revisions that still refer to it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectSourceLifecycle {
    Active,
    Retired,
}

/// Publication state makes the absence of a current revision explicit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", content = "revision", rename_all = "snake_case")]
pub enum EffectPublication {
    Unpublished,
    Published(EffectRevisionId),
}

/// A stable Effect source whose published revision may change over time.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectSource {
    pub identity: EffectSourceIdentity,
    pub kind: EffectKind,
    pub key: SkillSourceKey,
    pub lifecycle: EffectSourceLifecycle,
    pub publication: EffectPublication,
}

/// Safe stable explanation for an unavailable immutable revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StableReason(String);

impl StableReason {
    /// Refuses an empty reason because control and UI need a stable classification.
    pub fn parse(value: impl Into<String>) -> Result<Self, DesiredStateError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(DesiredStateError::EmptyStableReason);
        }
        Ok(Self(value))
    }

    /// Returns the safe stable representation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Availability is runtime input and therefore may change without mutating a revision's content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum RevisionAvailability {
    Available,
    Unavailable(StableReason),
}

/// Validated Skill definition independent of any Consumer output format.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkillDefinition {
    pub source: SkillSourceKey,
    pub skill_md_digest: Digest,
    pub package_fingerprint: Fingerprint,
    /// The source locator belongs to the source adapter and is never exposed in safe diagnostics.
    pub package_root: PathBuf,
}

/// Closed set of validated built-in definitions; adding a kind requires adding its typed branch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "definition", rename_all = "snake_case")]
pub enum ValidatedEffectDefinition {
    Skill(SkillDefinition),
}

impl ValidatedEffectDefinition {
    /// Returns the stable kind used to select the matching planner.
    pub fn kind(&self) -> EffectKind {
        match self {
            Self::Skill(_) => EffectKind::skill(),
        }
    }
}

/// One immutable, addressable version of an Effect source.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EffectRevision {
    pub identity: EffectRevisionId,
    pub source: EffectSourceIdentity,
    pub revision_key: SourceRevisionKey,
    pub definition: ValidatedEffectDefinition,
    pub digest: Digest,
    pub availability: RevisionAvailability,
}

/// Validated Skill parameters are intentionally empty until the Skill kind defines real inputs.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkillParameters {}

/// Closed set of kind-specific parameters, preventing arbitrary JSON branches in Effect Core.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "parameters", rename_all = "snake_case")]
pub enum ValidatedEffectParameters {
    Skill(SkillParameters),
}

impl ValidatedEffectParameters {
    /// Returns the kind whose planner can interpret these parameters.
    pub fn kind(&self) -> EffectKind {
        match self {
            Self::Skill(_) => EffectKind::skill(),
        }
    }
}

/// Strong capability predicate evaluated against one exact Consumer Revision.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CapabilityRequirement {
    pub effect_protocols: BTreeMap<EffectKind, u32>,
    pub materialization_contracts: BTreeSet<String>,
}

impl CapabilityRequirement {
    /// Returns whether a complete Consumer capability set satisfies every required capability.
    pub fn is_satisfied_by(&self, capabilities: &crate::CapabilitySet) -> bool {
        self.effect_protocols
            .iter()
            .all(|(kind, version)| capabilities.effect_protocols.get(kind) == Some(version))
            && self
                .materialization_contracts
                .is_subset(&capabilities.materialization_contracts)
    }
}

/// Explicit inclusion mode prevents an empty set from ambiguously meaning all or none.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "consumers", rename_all = "snake_case")]
pub enum TargetInclusion {
    AllEligible,
    Only(BTreeSet<ConsumerIdentity>),
}

/// Selects the eligible Consumer Targets that receive one Desired Effect.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetSelector {
    pub required_capabilities: CapabilityRequirement,
    pub include: TargetInclusion,
    pub exclude: BTreeSet<ConsumerIdentity>,
}

impl TargetSelector {
    /// Applies inclusion, exclusion, and capability rules without treating selection as readiness.
    pub fn selects(
        &self,
        consumer: &ConsumerIdentity,
        capabilities: &crate::CapabilitySet,
    ) -> bool {
        let included = match &self.include {
            TargetInclusion::AllEligible => true,
            TargetInclusion::Only(consumers) => consumers.contains(consumer),
        };
        included
            && !self.exclude.contains(consumer)
            && self.required_capabilities.is_satisfied_by(capabilities)
    }
}

impl Default for TargetSelector {
    fn default() -> Self {
        Self {
            required_capabilities: CapabilityRequirement::default(),
            include: TargetInclusion::AllEligible,
            exclude: BTreeSet::new(),
        }
    }
}

/// One stable item of intent in a complete Desired State.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesiredEffect {
    pub identity: DesiredEffectIdentity,
    pub revision: EffectRevisionId,
    pub parameters: ValidatedEffectParameters,
    pub audience: TargetSelector,
}

/// Complete normalized Desired Effect set for one Scope generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesiredState {
    pub scope: EffectScopeId,
    pub generation: Generation,
    pub effects: BTreeMap<DesiredEffectIdentity, DesiredEffect>,
}

impl DesiredState {
    /// Builds a deterministic snapshot and rejects duplicate stable intent identities.
    pub fn normalized(
        scope: EffectScopeId,
        generation: Generation,
        effects: impl IntoIterator<Item = DesiredEffect>,
    ) -> Result<Self, DesiredStateError> {
        let mut normalized = BTreeMap::new();
        for effect in effects {
            let identity = effect.identity.clone();
            if normalized.insert(identity.clone(), effect).is_some() {
                return Err(DesiredStateError::DuplicateDesiredEffect(identity));
            }
        }
        Ok(Self {
            scope,
            generation,
            effects: normalized,
        })
    }
}

/// Reports violations in immutable source or complete Desired State construction.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum DesiredStateError {
    #[error("stable reason must not be empty")]
    EmptyStableReason,
    #[error("duplicate Desired Effect identity {0}")]
    DuplicateDesiredEffect(DesiredEffectIdentity),
}
