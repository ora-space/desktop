use super::{SkillPlanner, blocking_condition, digest_serializable};
use ora_effect::{
    ConditionOwner, ConditionRetry, ConditionSubject, DesiredEffectIdentity, EffectKind,
    EffectResourceId, MaterializationContract, PlannerError, PlanningResult, ProjectionDigest,
    ResourceRequirement, RevisionAvailability, TargetPlanningInput, TargetProjection,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

impl SkillPlanner {
    /// Projects Skill intent into one complete Target snapshot for the shared reconciler.
    pub(super) fn project_target_snapshot(
        input: TargetPlanningInput<'_>,
    ) -> Result<PlanningResult<TargetProjection>, PlannerError> {
        validate_target_input(&input)?;
        let owner = ConditionOwner::Target(input.target.identity.clone());
        let mut conditions = Vec::new();
        let mut selected = BTreeSet::new();

        for desired in input.desired.effects.values() {
            if input.target.lifecycle == ora_effect::TargetLifecycle::Retiring {
                continue;
            }
            if !desired.audience.selects(
                &input.target.consumer,
                &input.consumer_revision.capabilities,
            ) {
                continue;
            }
            // A kind planner owns only its kind. The composite planner combines these selections
            // into one Target projection, so routing every selected Effect to every Resource would
            // feed MCP revisions into the Skill directory planner.
            if desired.parameters.kind() != EffectKind::skill() {
                continue;
            }
            let Some(revision) = input.revisions.get(&desired.revision) else {
                conditions.push(blocking_condition(
                    owner.clone(),
                    ConditionSubject::DesiredEffect(desired.identity.clone()),
                    "revision_missing",
                    input.desired.generation,
                    "The selected immutable Effect revision is unavailable.",
                    ConditionRetry::OnChange,
                ));
                continue;
            };
            if matches!(revision.availability, RevisionAvailability::Unavailable(_)) {
                conditions.push(blocking_condition(
                    owner.clone(),
                    ConditionSubject::DesiredEffect(desired.identity.clone()),
                    "revision_unavailable",
                    input.desired.generation,
                    "The selected immutable Effect revision is unavailable.",
                    ConditionRetry::OnChange,
                ));
                continue;
            }
            if revision.definition.kind() != desired.parameters.kind() {
                conditions.push(blocking_condition(
                    owner.clone(),
                    ConditionSubject::DesiredEffect(desired.identity.clone()),
                    "effect_kind_mismatch",
                    input.desired.generation,
                    "The Desired parameters do not match the selected definition kind.",
                    ConditionRetry::OnChange,
                ));
                continue;
            }
            if input
                .consumer_revision
                .capabilities
                .effect_protocols
                .get(&revision.definition.kind())
                != Some(&1)
            {
                conditions.push(blocking_condition(
                    owner.clone(),
                    ConditionSubject::DesiredEffect(desired.identity.clone()),
                    "unsupported_effect",
                    input.desired.generation,
                    "The Consumer Revision does not support this Effect protocol.",
                    ConditionRetry::OnChange,
                ));
                continue;
            }
            selected.insert(desired.identity.clone());
        }

        let mut requirements = BTreeMap::new();
        for binding in input.declaration.bindings.values() {
            if !input.resources.contains_key(&binding.resource) {
                conditions.push(blocking_condition(
                    owner.clone(),
                    ConditionSubject::Resource(binding.resource.clone()),
                    "resource_declaration_missing",
                    input.desired.generation,
                    "The Target binding refers to a Resource outside its declaration.",
                    ConditionRetry::OnChange,
                ));
                continue;
            }
            if !binding
                .accepts
                .is_satisfied_by(&input.consumer_revision.capabilities)
            {
                conditions.push(blocking_condition(
                    owner.clone(),
                    ConditionSubject::Resource(binding.resource.clone()),
                    "invalid_resource_binding",
                    input.desired.generation,
                    "The Target binding exceeds the Consumer Revision capabilities.",
                    ConditionRetry::OnChange,
                ));
                continue;
            }
            if binding.materialization_contract != MaterializationContract::skill_directory_v1() {
                continue;
            }
            let contract = binding.materialization_contract.clone();
            let draft = ResourceRequirementDigest {
                target: input.target.identity.as_str(),
                resource: binding.resource.as_str(),
                generation: input.desired.generation.value(),
                consumer_revision: input.consumer_revision.identity.as_str(),
                desired_effects: &selected,
                contract: &contract,
            };
            let requirement_digest = digest_serializable(&draft)?;
            requirements.insert(
                binding.resource.clone(),
                ResourceRequirement {
                    target: input.target.identity.clone(),
                    resource: binding.resource.clone(),
                    desired_effects: selected.clone(),
                    materialization_contract: contract,
                    digest: requirement_digest,
                },
            );
        }

        if !conditions.is_empty() {
            return Ok(PlanningResult::Blocked(conditions));
        }
        let draft = TargetProjectionDigest {
            target: input.target.identity.as_str(),
            generation: input.desired.generation.value(),
            consumer_revision: input.consumer_revision.identity.as_str(),
            desired_effects: &selected,
            requirements: &requirements,
        };
        let projection_digest = ProjectionDigest::new(digest_serializable(&draft)?);
        Ok(PlanningResult::Projected(TargetProjection {
            target: input.target.identity.clone(),
            generation: input.desired.generation,
            consumer_revision: input.consumer_revision.identity.clone(),
            desired_effects: selected,
            resource_requirements: requirements,
            digest: projection_digest,
        }))
    }
}

/// Verifies that all Target projection inputs describe the same Target and capability snapshot.
fn validate_target_input(input: &TargetPlanningInput<'_>) -> Result<(), PlannerError> {
    if input.desired.scope != input.target.scope {
        return Err(PlannerError::ScopeMismatch);
    }
    if input.target.consumer != input.consumer_revision.consumer {
        return Err(PlannerError::ConsumerMismatch);
    }
    if input.target.consumer_revision != input.consumer_revision.identity
        || input.declaration.consumer_revision != input.consumer_revision.identity
        || input.declaration.target != input.target.identity
    {
        return Err(PlannerError::ConsumerRevisionMismatch);
    }
    Ok(())
}

#[derive(Serialize)]
struct ResourceRequirementDigest<'a> {
    target: &'a str,
    resource: &'a str,
    generation: u64,
    consumer_revision: &'a str,
    desired_effects: &'a BTreeSet<DesiredEffectIdentity>,
    contract: &'a MaterializationContract,
}

#[derive(Serialize)]
struct TargetProjectionDigest<'a> {
    target: &'a str,
    generation: u64,
    consumer_revision: &'a str,
    desired_effects: &'a BTreeSet<DesiredEffectIdentity>,
    requirements: &'a BTreeMap<EffectResourceId, ResourceRequirement>,
}
