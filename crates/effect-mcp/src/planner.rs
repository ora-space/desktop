use ora_effect::{
    ConditionGeneration, ConditionImpact, ConditionOwner, ConditionProposal, ConditionRetry,
    ConditionSubject, Digest, EffectMutation, EffectPlanner, ExactPlannedState, ExactPreviousState,
    Generation, ManagedIdentity, ManagedItem, MaterializationContract, McpMaterializationInput,
    NativeResourceIdentity, OwnershipEvidence, PlannedMutation, PlannedResourceChange,
    PlannerError, PlanningResult, PreservedItem, ProjectionDigest, ResolvedMaterialization,
    ResourceObservation, ResourcePlan, ResourcePlanningInput, ResourceProjection,
    SafeConditionDetails, StableConditionCode, ValidatedEffectDefinition,
    VersionedMaterializationInput, VersionedResourceDescriptor,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

use crate::template::{McpAgentFormat, materialized_configuration};

/// Pure MCP Resource planner used behind the built-in composite planner seam.
#[derive(Clone, Copy, Debug, Default)]
pub struct McpPlanner;

impl EffectPlanner for McpPlanner {
    fn project_target(
        &self,
        _input: ora_effect::TargetPlanningInput<'_>,
    ) -> Result<PlanningResult<ora_effect::TargetProjection>, PlannerError> {
        unreachable!("the composite planner owns complete multi-kind Target projection")
    }

    fn plan_resource(
        &self,
        input: ResourcePlanningInput<'_>,
    ) -> Result<PlanningResult<ResourcePlan>, PlannerError> {
        plan_mcp_resource(input)
    }
}

/// Plans a whole shared MCP file, blocking the entire merge when any key is unsafe.
fn plan_mcp_resource(
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
    let mut contracts = BTreeSet::new();
    for requirement in input.requirements {
        if requirement.resource != input.resource.identity {
            return Err(PlannerError::RequirementResourceMismatch);
        }
        contributors.insert(requirement.target.clone());
        desired_ids.extend(requirement.desired_effects.iter().cloned());
        contracts.insert(requirement.materialization_contract.clone());
    }
    if contracts.len() != 1 {
        return Ok(PlanningResult::Blocked(vec![condition(
            owner,
            ConditionSubject::Resource(input.resource.identity.clone()),
            "materialization_contract_conflict",
            input.generation,
            "Target contributions require one exact MCP configuration contract.",
            ConditionRetry::OnChange,
        )]));
    }
    let Some(contract) = contracts.into_iter().next() else {
        return Ok(PlanningResult::Blocked(vec![condition(
            owner,
            ConditionSubject::Resource(input.resource.identity.clone()),
            "materialization_contract_conflict",
            input.generation,
            "Target contributions require one exact MCP configuration contract.",
            ConditionRetry::OnChange,
        )]));
    };
    let input_variant = match &contract {
        contract if *contract == MaterializationContract::opencode_mcp_config_v1() => {
            McpInputVariant::OpenCode
        }
        contract if *contract == MaterializationContract::claude_mcp_config_v1() => {
            McpInputVariant::Claude
        }
        _ => {
            return Ok(PlanningResult::Blocked(vec![condition(
                owner,
                ConditionSubject::Resource(input.resource.identity.clone()),
                "unsupported_materialization_contract",
                input.generation,
                "The MCP planner does not support the Resource materialization contract.",
                ConditionRetry::OnChange,
            )]));
        }
    };
    let workspace_root = match &input.resource.descriptor {
        VersionedResourceDescriptor::FilesystemFileV1(descriptor) => &descriptor.workspace_root,
        VersionedResourceDescriptor::FilesystemDirectoryV1(_) => {
            return Ok(PlanningResult::Blocked(vec![condition(
                owner,
                ConditionSubject::Resource(input.resource.identity.clone()),
                "unsupported_resource_descriptor",
                input.generation,
                "The MCP planner requires a shared file Resource.",
                ConditionRetry::OnChange,
            )]));
        }
    };
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
            conditions.push(condition(
                owner.clone(),
                ConditionSubject::DesiredEffect(desired_id.clone()),
                "revision_missing",
                input.generation,
                "The selected immutable MCP revision is unavailable.",
                ConditionRetry::OnChange,
            ));
            continue;
        };
        let ValidatedEffectDefinition::Mcp(definition) = &revision.definition else {
            conditions.push(condition(
                owner.clone(),
                ConditionSubject::DesiredEffect(desired_id.clone()),
                "effect_kind_mismatch",
                input.generation,
                "The selected Effect is not an MCP definition.",
                ConditionRetry::OnChange,
            ));
            continue;
        };
        let native_identity = NativeResourceIdentity::parse(definition.server_name.clone())?;
        if native_owners
            .insert(native_identity.clone(), desired_id.clone())
            .is_some()
        {
            conditions.push(condition(
                owner.clone(),
                ConditionSubject::DesiredEffect(desired_id.clone()),
                "native_identity_conflict",
                input.generation,
                "Multiple MCP plugins resolve to the same server name.",
                ConditionRetry::OnChange,
            ));
            continue;
        }
        let managed_identity = managed_by_desired
            .get(desired_id)
            .map(|managed| managed.identity.clone())
            .unwrap_or_else(|| ManagedIdentity::for_intent(&input.resource.identity, desired_id));
        let (format, environment) = match input_variant {
            McpInputVariant::OpenCode => (
                McpAgentFormat::OpenCode,
                definition.opencode_environment.clone(),
            ),
            McpInputVariant::Claude => (
                McpAgentFormat::Claude,
                definition.claude_environment.clone(),
            ),
        };
        let configuration = materialized_configuration(definition, format, workspace_root);
        let materialization = McpMaterializationInput {
            plugin_id: definition.plugin_id.clone(),
            server_name: definition.server_name.clone(),
            configuration_revision: definition.configuration_revision,
            configuration: configuration.clone(),
            environment,
        };
        let versioned = match input_variant {
            McpInputVariant::OpenCode => {
                VersionedMaterializationInput::OpenCodeMcpConfigV1(materialization)
            }
            McpInputVariant::Claude => {
                VersionedMaterializationInput::ClaudeMcpConfigV1(materialization)
            }
        };
        let fingerprint = FingerprintSource::configuration(&configuration)?;
        items.insert(
            managed_identity.clone(),
            ResolvedMaterialization {
                managed_identity,
                desired_effect: desired_id.clone(),
                revision: revision.identity.clone(),
                native_identity,
                fingerprint,
                contract: contract.clone(),
                input_digest: digest(&versioned)?,
                input: versioned,
            },
        );
    }
    for item in items.values() {
        let conflict = preserved.iter().any(|preserved| {
            preserved.native_identity == item.native_identity
                || preserved.native_identity.as_str()
                    == format!("jsonc:{}", item.native_identity.as_str())
                || preserved.native_identity.as_str()
                    == format!("sidecar:{}", item.native_identity.as_str())
        });
        if conflict {
            conditions.push(condition(
                owner.clone(),
                ConditionSubject::DesiredEffect(item.desired_effect.clone()),
                "preserved_item_conflict",
                input.generation,
                "A user entry, higher-priority JSONC entry, or mismatched sidecar owns this MCP server name.",
                ConditionRetry::OnChange,
            ));
        }
    }
    if !conditions.is_empty() {
        return Ok(PlanningResult::Blocked(conditions));
    }
    let projection_digest = ProjectionDigest::new(digest(&ProjectionDraft {
        resource: input.resource.identity.as_str(),
        generation: input.generation.value(),
        items: &items,
    })?);
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

#[derive(Clone, Copy)]
enum McpInputVariant {
    OpenCode,
    Claude,
}

struct FingerprintSource;

impl FingerprintSource {
    fn configuration(value: &Value) -> Result<ora_effect::Fingerprint, PlannerError> {
        serde_json::to_vec(value)
            .map(|bytes| ora_effect::Fingerprint::sha256(&bytes))
            .map_err(PlannerError::Serialize)
    }
}

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

fn plan_changes(
    generation: Generation,
    managed: &[ManagedItem],
    observed: &BTreeMap<ManagedIdentity, &ora_effect::ObservedItem>,
    projection: &ResourceProjection,
    owner: &ConditionOwner,
    conditions: &mut Vec<ConditionProposal>,
) -> Vec<PlannedResourceChange> {
    let mut changes = Vec::new();
    for managed_item in managed {
        let current = observed.get(&managed_item.identity).copied();
        if let Some(current) = current
            && current.fingerprint != managed_item.fingerprint
        {
            conditions.push(condition(
                owner.clone(),
                ConditionSubject::ManagedItem(managed_item.identity.clone()),
                "managed_item_drift",
                generation,
                "An Ora-managed MCP entry drifted and cannot be overwritten safely.",
                ConditionRetry::Manual,
            ));
            continue;
        }
        let desired = projection.items.get(&managed_item.identity);
        match (current, desired) {
            (None, None) => changes.push(PlannedResourceChange::ForgetMissing(
                managed_item.identity.clone(),
            )),
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
                })))
            }
            (None, Some(desired)) => changes.push(create_change(desired)),
            (Some(current), Some(desired)) => {
                if current.fingerprint != desired.fingerprint
                    || managed_item.applied_revision != desired.revision
                {
                    changes.push(PlannedResourceChange::Mutate(Box::new(PlannedMutation {
                        managed_identity: managed_item.identity.clone(),
                        desired_effect: Some(desired.desired_effect.clone()),
                        mutation: EffectMutation::Update,
                        expected: ExactPreviousState::Present {
                            native_identity: current.native_identity.clone(),
                            fingerprint: current.fingerprint.clone(),
                            managed_identity: managed_item.identity.clone(),
                        },
                        planned: ExactPlannedState::Present {
                            native_identity: desired.native_identity.clone(),
                            fingerprint: desired.fingerprint.clone(),
                            managed_identity: managed_item.identity.clone(),
                        },
                        input: Some(desired.input.clone()),
                    })));
                }
            }
        }
    }
    let existing = managed
        .iter()
        .map(|item| item.identity.clone())
        .collect::<BTreeSet<_>>();
    changes.extend(
        projection
            .items
            .values()
            .filter(|desired| !existing.contains(&desired.managed_identity))
            .map(create_change),
    );
    changes
}

fn create_change(desired: &ResolvedMaterialization) -> PlannedResourceChange {
    PlannedResourceChange::Mutate(Box::new(PlannedMutation {
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
    }))
}

fn condition(
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

fn digest(value: &impl Serialize) -> Result<Digest, PlannerError> {
    serde_json::to_vec(value)
        .map(|bytes| Digest::sha256(&bytes))
        .map_err(PlannerError::Serialize)
}

#[derive(Serialize)]
struct ProjectionDraft<'a> {
    resource: &'a str,
    generation: u64,
    items: &'a BTreeMap<ManagedIdentity, ResolvedMaterialization>,
}
