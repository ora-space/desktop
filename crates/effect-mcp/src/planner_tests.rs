use crate::{McpConfigResourceAdapter, McpPlanner};
use ora_domain::{PluginId, WorkspaceId};
use ora_effect::{
    DesiredEffect, DesiredEffectIdentity, Digest, EffectMutation, EffectPlanner, EffectResource,
    EffectResourceId, EffectRevision, EffectRevisionId, EffectScopeId, EffectSourceIdentity,
    EffectTargetId, FilesystemFileDescriptor, Fingerprint, Generation, LocalTimestamp,
    ManagedIdentity, ManagedItem, MaterializationContract, MaterializationFormat, McpParameters,
    McpTemplateDefinition, NativeResourceIdentity, ObservedItem, OwnershipEvidence,
    PlannedResourceChange, PlanningResult, ReconcileAttemptId, ResourceAdapter,
    ResourceAdapterIdentity, ResourceKey, ResourceLifecycle, ResourceObservation, ResourcePath,
    ResourcePlanningInput, ResourceRequirement, RevisionAvailability, SourceRevisionKey,
    TargetSelector, ValidatedEffectDefinition, ValidatedEffectParameters,
    VersionedResourceDescriptor,
};
use pretty_assertions::assert_eq;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

struct PlannerFixture {
    resource: EffectResource,
    desired: DesiredEffect,
    revision: EffectRevision,
    requirement: ResourceRequirement,
}

/// Builds one deterministic OpenCode MCP planning fixture rooted at the requested workspace.
fn fixture(workspace_root: &Path) -> PlannerFixture {
    let resource = EffectResource {
        identity: EffectResourceId::new("resource-mcp"),
        scope: EffectScopeId::Workspace(WorkspaceId::new("workspace")),
        resource_key: ResourceKey::parse("filesystem-file:.opencode/opencode.json")
            .expect("resource key"),
        adapter: ResourceAdapterIdentity::parse("ora/json-file-merge").expect("adapter identity"),
        descriptor: VersionedResourceDescriptor::FilesystemFileV1(FilesystemFileDescriptor {
            workspace_root: workspace_root.to_path_buf(),
            relative_path: ResourcePath::parse(".opencode/opencode.json").expect("config path"),
            ownership_relative_path: ResourcePath::parse(".opencode/.ora-mcp-managed.json")
                .expect("sidecar path"),
        }),
        format: MaterializationFormat::opencode_mcp_config_v1(),
        lifecycle: ResourceLifecycle::Active,
    };
    let revision_id = EffectRevisionId::new("revision-mcp");
    let desired = DesiredEffect {
        identity: DesiredEffectIdentity::new("desired-mcp"),
        revision: revision_id.clone(),
        parameters: ValidatedEffectParameters::Mcp(McpParameters::default()),
        audience: TargetSelector::default(),
    };
    let revision = EffectRevision {
        identity: revision_id,
        source: EffectSourceIdentity::new("source-mcp"),
        revision_key: SourceRevisionKey::parse("1").expect("revision key"),
        definition: ValidatedEffectDefinition::Mcp(McpTemplateDefinition {
            plugin_id: PluginId::parse("official/example.mcp").expect("plugin id"),
            server_name: "ora-example".to_string(),
            configuration_revision: 1,
            opencode: json!({"type":"remote","url":"https://example.test/mcp"}),
            claude: json!({"type":"http","url":"https://example.test/mcp"}),
            opencode_environment: BTreeMap::new(),
            claude_environment: BTreeMap::new(),
        }),
        digest: Digest::sha256(b"revision"),
        availability: RevisionAvailability::Available,
    };
    let requirement = ResourceRequirement {
        target: EffectTargetId::new("target-agent"),
        resource: resource.identity.clone(),
        desired_effects: BTreeSet::from([desired.identity.clone()]),
        materialization_contract: MaterializationContract::opencode_mcp_config_v1(),
        digest: Digest::sha256(b"requirement"),
    };
    PlannerFixture {
        resource,
        desired,
        revision,
        requirement,
    }
}

/// Invokes the pure planner from complete desired, revision, ledger, and observation snapshots.
fn plan(
    fixture: &PlannerFixture,
    requirement: &ResourceRequirement,
    desired_effects: BTreeMap<DesiredEffectIdentity, DesiredEffect>,
    managed: &[ManagedItem],
    observed: &ResourceObservation,
) -> PlanningResult<ora_effect::ResourcePlan> {
    McpPlanner
        .plan_resource(ResourcePlanningInput {
            resource: &fixture.resource,
            generation: Generation::new(1),
            requirements: std::slice::from_ref(requirement),
            desired_effects: &desired_effects,
            revisions: &BTreeMap::from([(
                fixture.revision.identity.clone(),
                fixture.revision.clone(),
            )]),
            managed,
            observed,
        })
        .expect("MCP planning")
}

/// Produces an empty external observation for the fixture Resource.
fn empty_observation(fixture: &PlannerFixture) -> ResourceObservation {
    ResourceObservation {
        resource: fixture.resource.identity.clone(),
        items: BTreeMap::new(),
        fingerprint: Fingerprint::sha256(b"empty"),
    }
}

/// Converts one resolved projection item into the durable ownership ledger row it would create.
fn managed_item(item: &ora_effect::ResolvedMaterialization) -> ManagedItem {
    ManagedItem {
        identity: item.managed_identity.clone(),
        resource: EffectResourceId::new("resource-mcp"),
        desired_effect: item.desired_effect.clone(),
        applied_revision: item.revision.clone(),
        native_identity: item.native_identity.clone(),
        fingerprint: item.fingerprint.clone(),
        applied_generation: Generation::new(1),
    }
}

/// Exercises create, replay, no-op convergence, and owned deletion against a commented user file.
#[test]
fn shared_file_lifecycle_preserves_user_jsonc_and_replays_idempotently() {
    let directory = tempfile::tempdir().expect("temporary workspace");
    let fixture = fixture(directory.path());
    let config_path = directory.path().join(".opencode").join("opencode.json");
    fs::create_dir_all(config_path.parent().expect("config parent")).expect("create config parent");
    fs::write(
        &config_path,
        "{\n  // user-owned\n  \"theme\": \"dark\",\n  \"mcp\": {\n    \"user\": {\"type\":\"remote\"},\n  },\n}\n",
    )
    .expect("write user config");
    let adapter = McpConfigResourceAdapter;
    let observed = adapter
        .observe(&fixture.resource)
        .expect("observe user config");
    let PlanningResult::Projected(created) = plan(
        &fixture,
        &fixture.requirement,
        BTreeMap::from([(fixture.desired.identity.clone(), fixture.desired.clone())]),
        &[],
        &observed,
    ) else {
        panic!("an unowned different server must not block creation");
    };
    let PlannedResourceChange::Mutate(mutation) = &created.changes[0] else {
        panic!("creation must produce a mutation");
    };
    assert_eq!(mutation.mutation, EffectMutation::Create);
    let operation = adapter
        .prepare_operation(
            &fixture.resource,
            ReconcileAttemptId::new("attempt-create"),
            Generation::new(1),
            /*sequence*/ 0,
            (**mutation).clone(),
            LocalTimestamp::from_millis(1),
        )
        .expect("prepare create")
        .operation;
    adapter.apply(&operation).expect("apply create");
    adapter.apply(&operation).expect("replay create");
    adapter.verify(&operation).expect("verify create");
    let source = fs::read_to_string(&config_path).expect("read merged config");
    assert!(source.contains("// user-owned"));
    assert!(source.contains("\"user\""));
    assert!(source.contains("\"ora-example\""));

    let projected_item = created
        .projection
        .items
        .values()
        .next()
        .expect("projected item")
        .clone();
    let managed = managed_item(&projected_item);
    let observed = adapter
        .observe(&fixture.resource)
        .expect("observe managed config");
    let PlanningResult::Projected(current) = plan(
        &fixture,
        &fixture.requirement,
        BTreeMap::from([(fixture.desired.identity.clone(), fixture.desired.clone())]),
        std::slice::from_ref(&managed),
        &observed,
    ) else {
        panic!("matching file and sidecar must be current");
    };
    assert_eq!(current.changes, Vec::new());

    let mut retiring_requirement = fixture.requirement.clone();
    retiring_requirement.desired_effects.clear();
    let PlanningResult::Projected(retiring) = plan(
        &fixture,
        &retiring_requirement,
        BTreeMap::new(),
        std::slice::from_ref(&managed),
        &observed,
    ) else {
        panic!("retirement must produce an owned deletion");
    };
    let PlannedResourceChange::Mutate(mutation) = &retiring.changes[0] else {
        panic!("retirement must mutate the shared file");
    };
    assert_eq!(mutation.mutation, EffectMutation::Delete);
    let operation = adapter
        .prepare_operation(
            &fixture.resource,
            ReconcileAttemptId::new("attempt-delete"),
            Generation::new(1),
            /*sequence*/ 1,
            (**mutation).clone(),
            LocalTimestamp::from_millis(2),
        )
        .expect("prepare delete")
        .operation;
    adapter.apply(&operation).expect("apply delete");
    adapter.verify(&operation).expect("verify delete");
    let source = fs::read_to_string(&config_path).expect("read retired config");
    assert!(source.contains("// user-owned"));
    assert!(source.contains("\"user\""));
    assert!(!source.contains("\"ora-example\""));
}

/// Blocks user keys, higher-priority JSONC keys, and sidecar-only claims for the desired name.
#[test]
fn name_and_ownership_collisions_block_the_whole_resource() {
    let directory = tempfile::tempdir().expect("temporary workspace");
    let fixture = fixture(directory.path());
    for native_identity in ["ora-example", "jsonc:ora-example", "sidecar:ora-example"] {
        let native_identity =
            NativeResourceIdentity::parse(native_identity).expect("native identity");
        let observed = ResourceObservation {
            resource: fixture.resource.identity.clone(),
            items: BTreeMap::from([(
                native_identity.clone(),
                ObservedItem {
                    native_identity,
                    fingerprint: Fingerprint::sha256(b"external"),
                    ownership_evidence: OwnershipEvidence::NoOwnershipEvidence,
                },
            )]),
            fingerprint: Fingerprint::sha256(b"observation"),
        };
        let PlanningResult::Blocked(conditions) = plan(
            &fixture,
            &fixture.requirement,
            BTreeMap::from([(fixture.desired.identity.clone(), fixture.desired.clone())]),
            &[],
            &observed,
        ) else {
            panic!("external ownership evidence must block the shared file");
        };
        assert_eq!(conditions[0].code.as_str(), "preserved_item_conflict");
    }
}

/// Refuses to overwrite an owned entry whose bytes no longer match the durable ledger.
#[test]
fn managed_entry_drift_requires_manual_recovery() {
    let directory = tempfile::tempdir().expect("temporary workspace");
    let fixture = fixture(directory.path());
    let PlanningResult::Projected(initial) = plan(
        &fixture,
        &fixture.requirement,
        BTreeMap::from([(fixture.desired.identity.clone(), fixture.desired.clone())]),
        &[],
        &empty_observation(&fixture),
    ) else {
        panic!("initial projection must succeed");
    };
    let item = initial
        .projection
        .items
        .values()
        .next()
        .expect("projected item");
    let managed = managed_item(item);
    let observed = ResourceObservation {
        resource: fixture.resource.identity.clone(),
        items: BTreeMap::from([(
            item.native_identity.clone(),
            ObservedItem {
                native_identity: item.native_identity.clone(),
                fingerprint: Fingerprint::sha256(b"drifted"),
                ownership_evidence: OwnershipEvidence::Claims(item.managed_identity.clone()),
            },
        )]),
        fingerprint: Fingerprint::sha256(b"observation"),
    };
    let PlanningResult::Blocked(conditions) = plan(
        &fixture,
        &fixture.requirement,
        BTreeMap::from([(fixture.desired.identity.clone(), fixture.desired.clone())]),
        std::slice::from_ref(&managed),
        &observed,
    ) else {
        panic!("managed drift must block mutation");
    };
    assert_eq!(conditions[0].code.as_str(), "managed_item_drift");
}

/// Treats a sidecar claim as preserved when it does not match Core's exact Managed Item ledger.
#[test]
fn mismatched_sidecar_claim_does_not_grant_mutation_authority() {
    let directory = tempfile::tempdir().expect("temporary workspace");
    let fixture = fixture(directory.path());
    let native_identity = NativeResourceIdentity::parse("ora-example").expect("native identity");
    let observed = ResourceObservation {
        resource: fixture.resource.identity.clone(),
        items: BTreeMap::from([(
            native_identity.clone(),
            ObservedItem {
                native_identity,
                fingerprint: Fingerprint::sha256(b"claimed"),
                ownership_evidence: OwnershipEvidence::Claims(ManagedIdentity::new("forged")),
            },
        )]),
        fingerprint: Fingerprint::sha256(b"observation"),
    };
    let PlanningResult::Blocked(conditions) = plan(
        &fixture,
        &fixture.requirement,
        BTreeMap::from([(fixture.desired.identity.clone(), fixture.desired.clone())]),
        &[],
        &observed,
    ) else {
        panic!("a forged sidecar identity must be preserved and block the name");
    };
    assert_eq!(conditions[0].code.as_str(), "preserved_item_conflict");
}
