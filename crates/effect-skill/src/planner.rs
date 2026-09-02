use ora_effect::{
    ConditionGeneration, ConditionImpact, ConditionOwner, ConditionProposal, ConditionRetry,
    ConditionSubject, Digest, EffectMutation, EffectPlanner, ExactPlannedState, ExactPreviousState,
    Generation, ManagedIdentity, ManagedItem, MaterializationContract, NativeResourceIdentity,
    OwnershipEvidence, PlannedMutation, PlannedResourceChange, PlannerError, PlanningResult,
    PreservedItem, ProjectionDigest, ResolvedMaterialization, ResourceObservation, ResourcePlan,
    ResourcePlanningInput, ResourceProjection, SafeConditionDetails, SkillMaterializationInput,
    StableConditionCode, VersionedMaterializationInput,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

mod target;

/// First Effect-kind planner for Skill definitions and filesystem directory Resources.
#[derive(Clone, Copy, Debug, Default)]
pub struct SkillPlanner;

impl EffectPlanner for SkillPlanner {
    fn project_target(
        &self,
        input: ora_effect::TargetPlanningInput<'_>,
    ) -> Result<PlanningResult<ora_effect::TargetProjection>, PlannerError> {
        Self::project_target_snapshot(input)
    }

    fn plan_resource(
        &self,
        input: ResourcePlanningInput<'_>,
    ) -> Result<PlanningResult<ResourcePlan>, PlannerError> {
        if input.observed.resource != input.resource.identity {
            return Err(PlannerError::ObservationResourceMismatch);
        }
        let owner = ConditionOwner::Resource(input.resource.identity.clone());
        let (preserved, observed_managed) = classify_observation(input.managed, input.observed);
        let mut conditions = Vec::new();
        let mut contributors = BTreeSet::new();
        let mut desired_ids = BTreeSet::new();
        let mut materialization_contracts = BTreeSet::new();
        for requirement in input.requirements {
            if requirement.resource != input.resource.identity {
                return Err(PlannerError::RequirementResourceMismatch);
            }
            contributors.insert(requirement.target.clone());
            desired_ids.extend(requirement.desired_effects.iter().cloned());
            materialization_contracts.insert(requirement.materialization_contract.clone());
        }
        if materialization_contracts.len() > 1 {
            return Ok(PlanningResult::Blocked(vec![blocking_condition(
                owner,
                ConditionSubject::Resource(input.resource.identity.clone()),
                "materialization_contract_conflict",
                input.generation,
                "Target contributions require incompatible Resource materialization contracts.",
                ConditionRetry::OnChange,
            )]));
        }
        let materialization_contract = materialization_contracts
            .into_iter()
            .next()
            .unwrap_or_else(MaterializationContract::skill_directory_v1);
        if materialization_contract != MaterializationContract::skill_directory_v1() {
            return Ok(PlanningResult::Blocked(vec![blocking_condition(
                owner,
                ConditionSubject::Resource(input.resource.identity.clone()),
                "unsupported_materialization_contract",
                input.generation,
                "The Skill planner does not support the Resource materialization contract.",
                ConditionRetry::OnChange,
            )]));
        }

        let managed_by_desired = input
            .managed
            .iter()
            .map(|managed| (managed.desired_effect.clone(), managed))
            .collect::<BTreeMap<_, _>>();
        let mut native_owners = BTreeMap::new();
        let mut items = BTreeMap::new();
        for desired_id in &desired_ids {
            let Some(desired) = input.desired_effects.get(desired_id) else {
                return Err(PlannerError::DesiredEffectMissing(desired_id.clone()));
            };
            let Some(revision) = input.revisions.get(&desired.revision) else {
                conditions.push(blocking_condition(
                    owner.clone(),
                    ConditionSubject::DesiredEffect(desired_id.clone()),
                    "revision_missing",
                    input.generation,
                    "The selected immutable Effect revision is unavailable.",
                    ConditionRetry::OnChange,
                ));
                continue;
            };
            let ora_effect::ValidatedEffectDefinition::Skill(definition) = &revision.definition;
            let native_identity =
                NativeResourceIdentity::parse(definition.source.name.canonical())?;
            if let Some(previous) =
                native_owners.insert(native_identity.clone(), desired_id.clone())
                && previous != *desired_id
            {
                conditions.push(blocking_condition(
                    owner.clone(),
                    ConditionSubject::DesiredEffect(desired_id.clone()),
                    "native_identity_conflict",
                    input.generation,
                    "Multiple Desired Effects resolve to the same native Resource identity.",
                    ConditionRetry::OnChange,
                ));
                continue;
            }
            let managed_identity = managed_by_desired
                .get(desired_id)
                .map(|managed| managed.identity.clone())
                .unwrap_or_else(|| {
                    ManagedIdentity::for_intent(&input.resource.identity, desired_id)
                });
            let materialization_input =
                VersionedMaterializationInput::SkillDirectoryV1(SkillMaterializationInput {
                    name: definition.source.name.clone(),
                    source: definition.source.clone(),
                    package_root: definition.package_root.clone(),
                    skill_md_digest: definition.skill_md_digest.clone(),
                    package_fingerprint: definition.package_fingerprint.clone(),
                });
            items.insert(
                managed_identity.clone(),
                ResolvedMaterialization {
                    managed_identity,
                    desired_effect: desired_id.clone(),
                    revision: revision.identity.clone(),
                    native_identity,
                    fingerprint: definition.package_fingerprint.clone(),
                    contract: materialization_contract.clone(),
                    input_digest: digest_serializable(&materialization_input)?,
                    input: materialization_input,
                },
            );
        }

        let preserved_by_native = preserved
            .iter()
            .map(|item| (item.native_identity.clone(), item))
            .collect::<BTreeMap<_, _>>();
        for item in items.values() {
            if preserved_by_native.contains_key(&item.native_identity) {
                conditions.push(blocking_condition(
                    owner.clone(),
                    ConditionSubject::DesiredEffect(item.desired_effect.clone()),
                    "preserved_item_conflict",
                    input.generation,
                    "A Preserved Item already occupies the required native identity.",
                    ConditionRetry::OnChange,
                ));
            }
        }
        if !conditions.is_empty() {
            return Ok(PlanningResult::Blocked(conditions));
        }

        let projection_draft = ResourceProjectionDigest {
            resource: input.resource.identity.as_str(),
            generation: input.generation.value(),
            contributors: &contributors,
            items: &items,
        };
        let projection_digest = ProjectionDigest::new(digest_serializable(&projection_draft)?);
        let projection = ResourceProjection {
            resource: input.resource.identity.clone(),
            generation: input.generation,
            contributors,
            items,
            digest: projection_digest,
        };
        let changes = plan_changes(
            input.generation,
            input.managed,
            &observed_managed,
            &projection,
            &owner,
            &mut conditions,
        );
        if !conditions.is_empty() {
            return Ok(PlanningResult::Blocked(conditions));
        }
        Ok(PlanningResult::Projected(ResourcePlan {
            projection,
            preserved,
            changes,
        }))
    }
}

/// Separates exact ledger matches from Preserved Items without treating marker claims as ownership.
fn classify_observation<'a>(
    managed: &'a [ManagedItem],
    observation: &'a ResourceObservation,
) -> (
    Vec<PreservedItem>,
    BTreeMap<ManagedIdentity, &'a ora_effect::ObservedItem>,
) {
    let ledger = managed
        .iter()
        .map(|item| (item.identity.clone(), item))
        .collect::<BTreeMap<_, _>>();
    let mut preserved = Vec::new();
    let mut matched = BTreeMap::new();
    for observed in observation.items.values() {
        let exact = match &observed.ownership_evidence {
            OwnershipEvidence::Claims(identity) => ledger.get(identity).filter(|managed| {
                managed.resource == observation.resource
                    && managed.native_identity == observed.native_identity
            }),
            OwnershipEvidence::NoOwnershipEvidence => None,
        };
        if let Some(managed) = exact {
            matched.insert(managed.identity.clone(), observed);
        } else {
            preserved.push(PreservedItem {
                resource: observation.resource.clone(),
                native_identity: observed.native_identity.clone(),
                fingerprint: observed.fingerprint.clone(),
            });
        }
    }
    (preserved, matched)
}

/// Plans only ledger-authorized mutations and reports drift instead of guessing external state.
fn plan_changes(
    generation: Generation,
    managed: &[ManagedItem],
    observed: &BTreeMap<ManagedIdentity, &ora_effect::ObservedItem>,
    projection: &ResourceProjection,
    owner: &ConditionOwner,
    conditions: &mut Vec<ConditionProposal>,
) -> Vec<PlannedResourceChange> {
    let desired_by_identity = &projection.items;
    let mut changes = Vec::new();
    for managed_item in managed {
        let current = observed.get(&managed_item.identity).copied();
        if let Some(current) = current
            && current.fingerprint != managed_item.fingerprint
        {
            conditions.push(blocking_condition(
                owner.clone(),
                ConditionSubject::ManagedItem(managed_item.identity.clone()),
                "managed_item_drift",
                generation,
                "A Managed Item changed outside Ora and cannot be overwritten safely.",
                ConditionRetry::Manual,
            ));
            continue;
        }
        let desired = desired_by_identity.get(&managed_item.identity);
        match (current, desired) {
            (None, None) => {
                changes.push(PlannedResourceChange::ForgetMissing(
                    managed_item.identity.clone(),
                ));
            }
            (Some(current), None) => {
                changes.push(PlannedResourceChange::Mutate(Box::new(PlannedMutation {
                    managed_identity: managed_item.identity.clone(),
                    desired_effect: None,
                    mutation: EffectMutation::Delete,
                    expected: ExactPreviousState::Present {
                        native_identity: current.native_identity.clone(),
                        fingerprint: current.fingerprint.clone(),
                        managed_identity: managed_item.identity.clone(),
                    },
                    planned: ExactPlannedState::Missing,
                    input: None,
                })));
            }
            (None, Some(desired)) => {
                changes.push(materialize_change(
                    managed_item,
                    desired,
                    EffectMutation::Create,
                    ExactPreviousState::Missing,
                ));
            }
            (Some(current), Some(desired)) => {
                if current.fingerprint == desired.fingerprint
                    && managed_item.applied_revision == desired.revision
                    && managed_item.native_identity == desired.native_identity
                {
                    continue;
                }
                let mutation = if managed_item.native_identity == desired.native_identity {
                    EffectMutation::Update
                } else {
                    EffectMutation::Replace
                };
                changes.push(materialize_change(
                    managed_item,
                    desired,
                    mutation,
                    ExactPreviousState::Present {
                        native_identity: current.native_identity.clone(),
                        fingerprint: current.fingerprint.clone(),
                        managed_identity: managed_item.identity.clone(),
                    },
                ));
            }
        }
    }

    let existing = managed
        .iter()
        .map(|item| item.identity.clone())
        .collect::<BTreeSet<_>>();
    for desired in desired_by_identity.values() {
        if existing.contains(&desired.managed_identity) {
            continue;
        }
        changes.push(PlannedResourceChange::Mutate(Box::new(PlannedMutation {
            managed_identity: desired.managed_identity.clone(),
            desired_effect: Some(desired.desired_effect.clone()),
            mutation: EffectMutation::Create,
            expected: ExactPreviousState::Missing,
            planned: ExactPlannedState::Present {
                native_identity: desired.native_identity.clone(),
                fingerprint: desired.fingerprint.clone(),
                managed_identity: desired.managed_identity.clone(),
            },
            input: Some(desired.input.clone()),
        })));
    }
    changes
}

/// Builds an update/replace/create proposal while retaining the stable ownership identity.
fn materialize_change(
    managed: &ManagedItem,
    desired: &ResolvedMaterialization,
    mutation: EffectMutation,
    expected: ExactPreviousState,
) -> PlannedResourceChange {
    PlannedResourceChange::Mutate(Box::new(PlannedMutation {
        managed_identity: managed.identity.clone(),
        desired_effect: Some(desired.desired_effect.clone()),
        mutation,
        expected,
        planned: ExactPlannedState::Present {
            native_identity: desired.native_identity.clone(),
            fingerprint: desired.fingerprint.clone(),
            managed_identity: managed.identity.clone(),
        },
        input: Some(desired.input.clone()),
    }))
}

/// Constructs one deterministic blocking fact without leaking adapter-specific details.
pub(super) fn blocking_condition(
    owner: ConditionOwner,
    subject: ConditionSubject,
    code: &'static str,
    generation: Generation,
    message: &'static str,
    retry: ConditionRetry,
) -> ConditionProposal {
    ConditionProposal {
        owner,
        subject,
        code: StableConditionCode::from_static(code),
        impact: ConditionImpact::Blocking,
        retry,
        generation: ConditionGeneration::At(generation),
        safe_details: SafeConditionDetails {
            message: message.to_string(),
            parameters: BTreeMap::new(),
        },
    }
}

/// Hashes deterministic serialized planner state into a projection content identity.
pub(super) fn digest_serializable(value: &impl Serialize) -> Result<Digest, PlannerError> {
    serde_json::to_vec(value)
        .map(|bytes| Digest::sha256(&bytes))
        .map_err(PlannerError::Serialize)
}

#[derive(Serialize)]
struct ResourceProjectionDigest<'a> {
    resource: &'a str,
    generation: u64,
    contributors: &'a BTreeSet<ora_effect::EffectTargetId>,
    items: &'a BTreeMap<ManagedIdentity, ResolvedMaterialization>,
}
