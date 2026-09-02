use crate::{
    ConsumerRevisionId, DesiredEffectIdentity, Digest, EffectResourceId, EffectRevisionId,
    EffectTargetId, Fingerprint, Generation, ManagedIdentity, MaterializationContract,
    NativeResourceIdentity, ProjectionDigest, SkillName, SkillSourceKey,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// One Target's complete contribution to one Resource at a generation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceRequirement {
    pub target: EffectTargetId,
    pub resource: EffectResourceId,
    pub desired_effects: BTreeSet<DesiredEffectIdentity>,
    pub materialization_contract: MaterializationContract,
    pub digest: Digest,
}

/// Deterministic complete view planned for one Target and exact Consumer Revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetProjection {
    pub target: EffectTargetId,
    pub generation: Generation,
    pub consumer_revision: ConsumerRevisionId,
    pub desired_effects: BTreeSet<DesiredEffectIdentity>,
    pub resource_requirements: BTreeMap<EffectResourceId, ResourceRequirement>,
    pub digest: ProjectionDigest,
}

/// Validated input for materializing one Skill directory into an exact Resource.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SkillMaterializationInput {
    pub name: SkillName,
    pub source: SkillSourceKey,
    pub package_root: PathBuf,
    pub skill_md_digest: Digest,
    pub package_fingerprint: Fingerprint,
}

/// Closed set of adapter-validated versioned materialization inputs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "input", rename_all = "snake_case")]
pub enum VersionedMaterializationInput {
    SkillDirectoryV1(SkillMaterializationInput),
}

/// One normalized Desired Effect result for an exact Resource and Consumer capability set.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResolvedMaterialization {
    pub managed_identity: ManagedIdentity,
    pub desired_effect: DesiredEffectIdentity,
    pub revision: EffectRevisionId,
    pub native_identity: NativeResourceIdentity,
    pub fingerprint: Fingerprint,
    pub contract: MaterializationContract,
    pub input_digest: Digest,
    pub input: VersionedMaterializationInput,
}

/// Complete merged state for one Resource after combining every active Target contribution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceProjection {
    pub resource: EffectResourceId,
    pub generation: Generation,
    pub contributors: BTreeSet<EffectTargetId>,
    pub items: BTreeMap<ManagedIdentity, ResolvedMaterialization>,
    pub digest: ProjectionDigest,
}

/// Durable ownership ledger for one external item inside a Resource.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManagedItem {
    pub identity: ManagedIdentity,
    pub resource: EffectResourceId,
    pub desired_effect: DesiredEffectIdentity,
    pub applied_revision: EffectRevisionId,
    pub native_identity: NativeResourceIdentity,
    pub fingerprint: Fingerprint,
    pub applied_generation: Generation,
}

/// Evidence reported by an adapter remains a claim until it matches the durable ledger exactly.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "identity", rename_all = "snake_case")]
pub enum OwnershipEvidence {
    NoOwnershipEvidence,
    Claims(ManagedIdentity),
}

/// Normalized external fact for one native Resource item.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ObservedItem {
    pub native_identity: NativeResourceIdentity,
    pub fingerprint: Fingerprint,
    pub ownership_evidence: OwnershipEvidence,
}

/// Complete observed Resource facts at one moment, without granting ownership.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ResourceObservation {
    pub resource: EffectResourceId,
    pub items: BTreeMap<NativeResourceIdentity, ObservedItem>,
    pub fingerprint: Fingerprint,
}

/// External item without an exact Managed ledger match and therefore outside mutation authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PreservedItem {
    pub resource: EffectResourceId,
    pub native_identity: NativeResourceIdentity,
    pub fingerprint: Fingerprint,
}
