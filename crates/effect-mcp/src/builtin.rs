use crate::{McpConfigResourceAdapter, McpPlanner};
use ora_effect::{
    ApplyReceipt, CleanupReceipt, ConditionGeneration, ConditionImpact, ConditionOwner,
    ConditionProposal, ConditionRetry, ConditionSubject, Digest, EffectKind, EffectPlanner,
    EffectResourceId, LocalTimestamp, MaterializationContract, PlannedMutation, PlannerError,
    PlanningResult, PreparedOperation, ProjectionDigest, ReconcileAttemptId, ResourceAdapter,
    ResourceAdapterError, ResourcePlan, ResourcePlanningInput, ResourceRequirement,
    RevisionAvailability, SafeConditionDetails, StableConditionCode, TargetPlanningInput,
    TargetProjection, VerificationReceipt, VersionedAdapterPlan,
};
use ora_effect_skill::{SkillDirectoryResourceAdapter, SkillPlanner};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// One pure planner for the complete built-in Effect kind set.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuiltinEffectPlanner;

impl EffectPlanner for BuiltinEffectPlanner {
    fn project_target(
        &self,
        input: TargetPlanningInput<'_>,
    ) -> Result<PlanningResult<TargetProjection>, PlannerError> {
        project_target(input)
    }

    fn plan_resource(
        &self,
        input: ResourcePlanningInput<'_>,
    ) -> Result<PlanningResult<ResourcePlan>, PlannerError> {
        match input.resource.format {
            ref format if *format == ora_effect::MaterializationFormat::skill_directory_v1() => {
                SkillPlanner.plan_resource(input)
            }
            ref format
                if *format == ora_effect::MaterializationFormat::opencode_mcp_config_v1()
                    || *format == ora_effect::MaterializationFormat::claude_mcp_config_v1() =>
            {
                McpPlanner.plan_resource(input)
            }
            _ => Ok(PlanningResult::Blocked(vec![condition(
                ConditionOwner::Resource(input.resource.identity.clone()),
                ConditionSubject::Resource(input.resource.identity.clone()),
                "unsupported_materialization_format",
                input.generation,
                "No built-in planner supports this Resource materialization format.",
            )])),
        }
    }
}

/// One statically dispatched Resource adapter for the complete built-in payload set.
#[derive(Clone, Copy, Debug, Default)]
pub struct BuiltinResourceAdapter;

impl ResourceAdapter for BuiltinResourceAdapter {
    fn prepare_operation(
        &self,
        resource: &ora_effect::EffectResource,
        attempt: ReconcileAttemptId,
        generation: ora_effect::Generation,
        sequence: u32,
        mutation: PlannedMutation,
        prepared_at: LocalTimestamp,
    ) -> Result<PreparedOperation, ResourceAdapterError> {
        match resource.format {
            ref format if *format == ora_effect::MaterializationFormat::skill_directory_v1() => {
                ResourceAdapter::prepare_operation(
                    &SkillDirectoryResourceAdapter,
                    resource,
                    attempt,
                    generation,
                    sequence,
                    mutation,
                    prepared_at,
                )
            }
            ref format
                if *format == ora_effect::MaterializationFormat::opencode_mcp_config_v1()
                    || *format == ora_effect::MaterializationFormat::claude_mcp_config_v1() =>
            {
                ResourceAdapter::prepare_operation(
                    &McpConfigResourceAdapter,
                    resource,
                    attempt,
                    generation,
                    sequence,
                    mutation,
                    prepared_at,
                )
            }
            _ => Err(ResourceAdapterError::new(std::io::Error::other(
                "unsupported built-in Resource format",
            ))),
        }
    }

    fn observe(
        &self,
        resource: &ora_effect::EffectResource,
    ) -> Result<ora_effect::ResourceObservation, ResourceAdapterError> {
        match resource.format {
            ref format if *format == ora_effect::MaterializationFormat::skill_directory_v1() => {
                SkillDirectoryResourceAdapter.observe(resource)
            }
            ref format
                if *format == ora_effect::MaterializationFormat::opencode_mcp_config_v1()
                    || *format == ora_effect::MaterializationFormat::claude_mcp_config_v1() =>
            {
                McpConfigResourceAdapter.observe(resource)
            }
            _ => Err(ResourceAdapterError::new(std::io::Error::other(
                "unsupported built-in Resource format",
            ))),
        }
    }

    fn apply(
        &self,
        operation: &ora_effect::EffectOperation,
    ) -> Result<ApplyReceipt, ResourceAdapterError> {
        match operation.payload() {
            VersionedAdapterPlan::FilesystemDirectoryV1(_) => {
                SkillDirectoryResourceAdapter.apply(operation)
            }
            VersionedAdapterPlan::JsonMergeV1(_) => McpConfigResourceAdapter.apply(operation),
        }
    }

    fn verify(
        &self,
        operation: &ora_effect::EffectOperation,
    ) -> Result<VerificationReceipt, ResourceAdapterError> {
        match operation.payload() {
            VersionedAdapterPlan::FilesystemDirectoryV1(_) => {
                SkillDirectoryResourceAdapter.verify(operation)
            }
            VersionedAdapterPlan::JsonMergeV1(_) => McpConfigResourceAdapter.verify(operation),
        }
    }

    fn cleanup(
        &self,
        artifact: &ora_effect::OperationArtifact,
    ) -> Result<CleanupReceipt, ResourceAdapterError> {
        SkillDirectoryResourceAdapter.cleanup(artifact)
    }
}

/// Projects all supported kinds once, then routes each kind only to accepting Resources.
fn project_target(
    input: TargetPlanningInput<'_>,
) -> Result<PlanningResult<TargetProjection>, PlannerError> {
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
    let owner = ConditionOwner::Target(input.target.identity.clone());
    let mut conditions = Vec::new();
    let mut selected = BTreeMap::new();
    if input.target.lifecycle == ora_effect::TargetLifecycle::Active {
        for desired in input.desired.effects.values() {
            if !desired.audience.selects(
                &input.target.consumer,
                &input.consumer_revision.capabilities,
            ) {
                continue;
            }
            let Some(revision) = input.revisions.get(&desired.revision) else {
                conditions.push(condition(
                    owner.clone(),
                    ConditionSubject::DesiredEffect(desired.identity.clone()),
                    "revision_missing",
                    input.desired.generation,
                    "The selected immutable Effect revision is unavailable.",
                ));
                continue;
            };
            if matches!(revision.availability, RevisionAvailability::Unavailable(_)) {
                conditions.push(condition(
                    owner.clone(),
                    ConditionSubject::DesiredEffect(desired.identity.clone()),
                    "revision_unavailable",
                    input.desired.generation,
                    "The selected immutable Effect revision is unavailable.",
                ));
                continue;
            }
            let kind = revision.definition.kind();
            if kind != desired.parameters.kind() {
                conditions.push(condition(
                    owner.clone(),
                    ConditionSubject::DesiredEffect(desired.identity.clone()),
                    "effect_kind_mismatch",
                    input.desired.generation,
                    "The Desired parameters do not match the selected definition kind.",
                ));
                continue;
            }
            if input
                .consumer_revision
                .capabilities
                .effect_protocols
                .get(&kind)
                != Some(&1)
            {
                conditions.push(condition(
                    owner.clone(),
                    ConditionSubject::DesiredEffect(desired.identity.clone()),
                    "unsupported_effect",
                    input.desired.generation,
                    "The Consumer Revision does not support this Effect protocol.",
                ));
                continue;
            }
            selected.insert(desired.identity.clone(), kind);
        }
    }
    let mut requirements = BTreeMap::new();
    let mut routed = BTreeSet::new();
    for binding in input.declaration.bindings.values() {
        if !input.resources.contains_key(&binding.resource) {
            conditions.push(condition(
                owner.clone(),
                ConditionSubject::Resource(binding.resource.clone()),
                "resource_declaration_missing",
                input.desired.generation,
                "The Target binding refers to a Resource outside its declaration.",
            ));
            continue;
        }
        let desired_effects = selected
            .iter()
            .filter(|(_, kind)| accepts_kind(binding, kind))
            .map(|(identity, _)| identity.clone())
            .collect::<BTreeSet<_>>();
        routed.extend(desired_effects.iter().cloned());
        let contract = binding.materialization_contract.clone();
        let digest = digest(&RequirementDraft {
            target: input.target.identity.as_str(),
            resource: binding.resource.as_str(),
            generation: input.desired.generation.value(),
            desired_effects: &desired_effects,
            contract: &contract,
        })?;
        requirements.insert(
            binding.resource.clone(),
            ResourceRequirement {
                target: input.target.identity.clone(),
                resource: binding.resource.clone(),
                desired_effects,
                materialization_contract: contract,
                digest,
            },
        );
    }
    for identity in selected.keys() {
        if !routed.contains(identity) {
            conditions.push(condition(
                owner.clone(),
                ConditionSubject::DesiredEffect(identity.clone()),
                "effect_resource_missing",
                input.desired.generation,
                "The Consumer supports this Effect kind but declares no compatible Resource.",
            ));
        }
    }
    if !conditions.is_empty() {
        return Ok(PlanningResult::Blocked(conditions));
    }
    let desired_effects = selected.into_keys().collect::<BTreeSet<_>>();
    let projection_digest = ProjectionDigest::new(digest(&TargetDraft {
        target: input.target.identity.as_str(),
        generation: input.desired.generation.value(),
        consumer_revision: input.consumer_revision.identity.as_str(),
        desired_effects: &desired_effects,
        requirements: &requirements,
    })?);
    Ok(PlanningResult::Projected(TargetProjection {
        target: input.target.identity.clone(),
        generation: input.desired.generation,
        consumer_revision: input.consumer_revision.identity.clone(),
        desired_effects,
        resource_requirements: requirements,
        digest: projection_digest,
    }))
}

fn accepts_kind(binding: &ora_effect::TargetResourceBinding, kind: &EffectKind) -> bool {
    binding.accepts.effect_protocols.get(kind) == Some(&1)
        && binding
            .accepts
            .materialization_contracts
            .contains(&binding.materialization_contract.capability_key())
}

fn condition(
    owner: ConditionOwner,
    subject: ConditionSubject,
    code: &'static str,
    generation: ora_effect::Generation,
    message: &'static str,
) -> ConditionProposal {
    ConditionProposal {
        owner,
        subject,
        code: StableConditionCode::from_static(code),
        impact: ConditionImpact::Blocking,
        retry: ConditionRetry::OnChange,
        generation: ConditionGeneration::At(generation),
        safe_details: SafeConditionDetails {
            message: message.to_string(),
            parameters: BTreeMap::new(),
        },
    }
}

fn digest(value: &impl Serialize) -> Result<Digest, PlannerError> {
    serde_json::to_vec(value)
        .map(|bytes| Digest::sha256(&bytes))
        .map_err(PlannerError::Serialize)
}

#[derive(Serialize)]
struct RequirementDraft<'a> {
    target: &'a str,
    resource: &'a str,
    generation: u64,
    desired_effects: &'a BTreeSet<ora_effect::DesiredEffectIdentity>,
    contract: &'a MaterializationContract,
}

#[derive(Serialize)]
struct TargetDraft<'a> {
    target: &'a str,
    generation: u64,
    consumer_revision: &'a str,
    desired_effects: &'a BTreeSet<ora_effect::DesiredEffectIdentity>,
    requirements: &'a BTreeMap<EffectResourceId, ResourceRequirement>,
}
