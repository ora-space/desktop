use crate::*;
use ora_domain::{Namespace, WorkspaceId};
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::fs;
use tempfile::TempDir;

#[derive(Clone, Copy)]
struct FixedIdentity;

impl ManagedIdentityGenerator for FixedIdentity {
    fn generate_managed_identity(&self) -> ManagedIdentity {
        ManagedIdentity::new("fresh")
    }
}

/// Builds one exact Local desired state with compact deterministic fixture values.
fn desired(name: &str, version: &str, manifest: &[u8]) -> (SkillSelectionKey, DesiredSkillState) {
    let name = SkillName::parse(name).unwrap_or_else(|error| panic!("parse name: {error}"));
    let source = SkillSource::Local {
        namespace: Namespace::local(),
        version: SourceVersion::parse(version)
            .unwrap_or_else(|error| panic!("parse version: {error}")),
    };
    let state = DesiredSkillState::try_new(SkillState {
        name: name.clone(),
        skill_md_digest: Digest::sha256(manifest),
        source,
    })
    .unwrap_or_else(|error| panic!("build desired: {error}"));
    (
        SkillSelectionKey::new(SourceKind::Local, Namespace::local(), name),
        state,
    )
}

/// Builds a ledger whose applied directory is represented by `fingerprint`.
fn managed(
    workspace_id: &WorkspaceId,
    surface_key: &SurfaceKey,
    identity: &str,
    desired: &(SkillSelectionKey, DesiredSkillState),
    fingerprint: &str,
    generation: u64,
) -> ManagedSkill {
    ManagedSkill {
        managed_identity: ManagedIdentity::new(identity),
        workspace_id: workspace_id.clone(),
        surface_key: surface_key.clone(),
        selection_key: desired.0.clone(),
        locator: desired.0.name.canonical().to_string(),
        target_name: desired.0.name.clone(),
        state: desired.1.clone(),
        applied_fingerprint: AppliedFingerprint::parse(fingerprint)
            .unwrap_or_else(|error| panic!("parse fingerprint: {error}")),
        applied_generation: Generation::new(generation),
    }
}

/// Returns a stable valid fingerprint with a repeated hex digit.
fn fingerprint(digit: char) -> String {
    format!("sha256:{}", digit.to_string().repeat(64))
}

#[test]
fn planner_creates_missing_desired_with_fresh_ownership() {
    let workspace_id = WorkspaceId::new("workspace");
    let surface_key = SurfaceKey::new("surface");
    let selected = desired("review", "1", b"manifest");
    let desired = BTreeMap::from([(selected.0.clone(), selected.1.clone())]);

    let plan = Planner::new(&FixedIdentity).plan(PlannerInput {
        surface_key: &surface_key,
        lifecycle: SurfaceLifecycle::Active,
        generation: Generation::new(1),
        desired: &desired,
        managed: &[],
        observed: &BTreeMap::new(),
        occurred_at: 10,
    });

    assert_eq!(
        plan,
        ReconcilePlan {
            generation: Generation::new(1),
            operations: vec![PlanOperation {
                locator: "review".to_string(),
                kind: PlanOperationKind::Create {
                    desired: selected.1,
                    managed_identity: ManagedIdentity::new("fresh"),
                },
                requires_filesystem_mutation: true,
            }],
            conditions: vec![],
        }
    );
    let _ = workspace_id;
}

#[test]
fn planner_never_overwrites_preserved_or_unproven_marker() {
    let selected = desired("review", "1", b"manifest");
    let desired = BTreeMap::from([(selected.0.clone(), selected.1)]);
    let surface_key = SurfaceKey::new("surface");

    for observation in [
        TargetObservation::Preserved,
        TargetObservation::Managed {
            marker_identity: ManagedIdentity::new("orphan"),
            fingerprint: AppliedFingerprint::parse(fingerprint('a'))
                .unwrap_or_else(|error| panic!("parse fingerprint: {error}")),
        },
    ] {
        let plan = Planner::new(&FixedIdentity).plan(PlannerInput {
            surface_key: &surface_key,
            lifecycle: SurfaceLifecycle::Active,
            generation: Generation::new(1),
            desired: &desired,
            managed: &[],
            observed: &BTreeMap::from([("review".to_string(), observation)]),
            occurred_at: 10,
        });
        assert!(plan.operations.is_empty());
        assert_eq!(plan.conditions.len(), 1);
    }
}

#[test]
fn planner_distinguishes_ownership_and_content_drift() {
    let workspace_id = WorkspaceId::new("workspace");
    let surface_key = SurfaceKey::new("surface");
    let selected = desired("review", "1", b"manifest");
    let ledger = managed(
        &workspace_id,
        &surface_key,
        "owned",
        &selected,
        &fingerprint('a'),
        1,
    );
    let desired = BTreeMap::from([(selected.0.clone(), selected.1)]);
    let cases = [
        (
            TargetObservation::Managed {
                marker_identity: ManagedIdentity::new("other"),
                fingerprint: ledger.applied_fingerprint.clone(),
            },
            ConditionReason::OwnershipConflict,
        ),
        (
            TargetObservation::Managed {
                marker_identity: ledger.managed_identity.clone(),
                fingerprint: AppliedFingerprint::parse(fingerprint('b'))
                    .unwrap_or_else(|error| panic!("parse fingerprint: {error}")),
            },
            ConditionReason::DriftConflict,
        ),
    ];

    for (observation, reason) in cases {
        let plan = Planner::new(&FixedIdentity).plan(PlannerInput {
            surface_key: &surface_key,
            lifecycle: SurfaceLifecycle::Active,
            generation: Generation::new(2),
            desired: &desired,
            managed: std::slice::from_ref(&ledger),
            observed: &BTreeMap::from([("review".to_string(), observation)]),
            occurred_at: 10,
        });
        assert!(plan.operations.is_empty());
        assert_eq!(plan.conditions[0].reason, reason);
    }
}

#[test]
fn planner_preserves_identity_for_update_and_replaces_identity_for_new_source() {
    let workspace_id = WorkspaceId::new("workspace");
    let surface_key = SurfaceKey::new("surface");
    let old = desired("review", "1", b"old");
    let updated = desired("review", "2", b"new");
    let ledger = managed(
        &workspace_id,
        &surface_key,
        "owned",
        &old,
        &fingerprint('a'),
        1,
    );
    let observation = BTreeMap::from([(
        "review".to_string(),
        TargetObservation::Managed {
            marker_identity: ledger.managed_identity.clone(),
            fingerprint: ledger.applied_fingerprint.clone(),
        },
    )]);
    let update_plan = Planner::new(&FixedIdentity).plan(PlannerInput {
        surface_key: &surface_key,
        lifecycle: SurfaceLifecycle::Active,
        generation: Generation::new(2),
        desired: &BTreeMap::from([(updated.0, updated.1)]),
        managed: std::slice::from_ref(&ledger),
        observed: &observation,
        occurred_at: 10,
    });
    assert!(matches!(
        &update_plan.operations[0].kind,
        PlanOperationKind::Update { previous, .. }
            if previous.managed_identity == ManagedIdentity::new("owned")
    ));

    let plugin_name =
        SkillName::parse("review").unwrap_or_else(|error| panic!("parse plugin name: {error}"));
    let plugin_key = SkillSelectionKey::new(
        SourceKind::Plugin,
        Namespace::new("publisher").unwrap_or_else(|error| panic!("parse namespace: {error}")),
        plugin_name.clone(),
    );
    let plugin_state = DesiredSkillState::try_new(SkillState {
        name: plugin_name,
        skill_md_digest: Digest::sha256(b"plugin"),
        source: SkillSource::Plugin {
            namespace: Namespace::new("publisher")
                .unwrap_or_else(|error| panic!("parse namespace: {error}")),
            version: SourceVersion::parse("1")
                .unwrap_or_else(|error| panic!("parse version: {error}")),
        },
    })
    .unwrap_or_else(|error| panic!("build plugin state: {error}"));
    let replace_plan = Planner::new(&FixedIdentity).plan(PlannerInput {
        surface_key: &surface_key,
        lifecycle: SurfaceLifecycle::Active,
        generation: Generation::new(2),
        desired: &BTreeMap::from([(plugin_key, plugin_state)]),
        managed: &[ledger],
        observed: &observation,
        occurred_at: 10,
    });
    assert!(matches!(
        &replace_plan.operations[0].kind,
        PlanOperationKind::Replace { managed_identity, .. }
            if managed_identity == &ManagedIdentity::new("fresh")
    ));
}

#[test]
fn planner_advances_unchanged_resources_without_filesystem_mutation() {
    let workspace_id = WorkspaceId::new("workspace");
    let surface_key = SurfaceKey::new("surface");
    let selected = desired("review", "1", b"same");
    let ledger = managed(
        &workspace_id,
        &surface_key,
        "owned",
        &selected,
        &fingerprint('a'),
        1,
    );
    let plan = Planner::new(&FixedIdentity).plan(PlannerInput {
        surface_key: &surface_key,
        lifecycle: SurfaceLifecycle::Active,
        generation: Generation::new(2),
        desired: &BTreeMap::from([(selected.0, selected.1)]),
        managed: std::slice::from_ref(&ledger),
        observed: &BTreeMap::from([(
            "review".to_string(),
            TargetObservation::Managed {
                marker_identity: ledger.managed_identity.clone(),
                fingerprint: ledger.applied_fingerprint.clone(),
            },
        )]),
        occurred_at: 10,
    });

    assert_eq!(plan.operations[0].requires_filesystem_mutation, false);
    assert!(matches!(
        plan.operations[0].kind,
        PlanOperationKind::AdvanceGeneration { .. }
    ));
}

#[test]
fn planner_blocks_locator_collisions_and_retires_only_owned_state() {
    let workspace_id = WorkspaceId::new("workspace");
    let surface_key = SurfaceKey::new("surface");
    let local = desired("review", "1", b"local");
    let plugin_name =
        SkillName::parse("REVIEW").unwrap_or_else(|error| panic!("parse plugin name: {error}"));
    let plugin_key = SkillSelectionKey::new(
        SourceKind::Plugin,
        Namespace::new("publisher").unwrap_or_else(|error| panic!("parse namespace: {error}")),
        plugin_name.clone(),
    );
    let plugin = DesiredSkillState::try_new(SkillState {
        name: plugin_name,
        skill_md_digest: Digest::sha256(b"plugin"),
        source: SkillSource::Plugin {
            namespace: Namespace::new("publisher")
                .unwrap_or_else(|error| panic!("parse namespace: {error}")),
            version: SourceVersion::parse("1")
                .unwrap_or_else(|error| panic!("parse version: {error}")),
        },
    })
    .unwrap_or_else(|error| panic!("build plugin: {error}"));
    let desired_map = BTreeMap::from([(local.0.clone(), local.1.clone()), (plugin_key, plugin)]);
    let collision = Planner::new(&FixedIdentity).plan(PlannerInput {
        surface_key: &surface_key,
        lifecycle: SurfaceLifecycle::Active,
        generation: Generation::new(1),
        desired: &desired_map,
        managed: &[],
        observed: &BTreeMap::new(),
        occurred_at: 10,
    });
    assert!(collision.operations.is_empty());
    assert_eq!(collision.conditions.len(), 2);

    let ledger = managed(
        &workspace_id,
        &surface_key,
        "owned",
        &local,
        &fingerprint('a'),
        1,
    );
    let retiring = Planner::new(&FixedIdentity).plan(PlannerInput {
        surface_key: &surface_key,
        lifecycle: SurfaceLifecycle::Retiring,
        generation: Generation::new(2),
        desired: &desired_map,
        managed: std::slice::from_ref(&ledger),
        observed: &BTreeMap::from([(
            "review".to_string(),
            TargetObservation::Managed {
                marker_identity: ledger.managed_identity.clone(),
                fingerprint: ledger.applied_fingerprint.clone(),
            },
        )]),
        occurred_at: 10,
    });
    assert!(matches!(
        retiring.operations[0].kind,
        PlanOperationKind::Delete { .. }
    ));
}

#[test]
fn merges_compatible_surface_consumers_and_rejects_format_conflicts() {
    let workspace_id = WorkspaceId::new("workspace");
    let path =
        SurfacePath::parse(".agents/skills").unwrap_or_else(|error| panic!("parse path: {error}"));
    let descriptors = [
        FilesystemSkillSurface {
            workspace_relative_path: path.clone(),
            materialization_format: MaterializationFormat::skill_directory_v1(),
            consumer: ConsumerId::new("codex"),
            coordination: ConsumerCoordination::WaitForIdleAndRestart,
        },
        FilesystemSkillSurface {
            workspace_relative_path: path.clone(),
            materialization_format: MaterializationFormat::skill_directory_v1(),
            consumer: ConsumerId::new("opencode"),
            coordination: ConsumerCoordination::Uninterrupted,
        },
    ];
    let merged = SurfaceDescriptorSet::merge(&workspace_id, descriptors)
        .unwrap_or_else(|error| panic!("merge descriptors: {error}"));
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].consumers.len(), 2);
    assert!(merged[0].requires_coordination());

    let conflict = SurfaceDescriptorSet::merge(
        &workspace_id,
        [
            FilesystemSkillSurface {
                workspace_relative_path: path.clone(),
                materialization_format: MaterializationFormat::skill_directory_v1(),
                consumer: ConsumerId::new("codex"),
                coordination: ConsumerCoordination::Uninterrupted,
            },
            FilesystemSkillSurface {
                workspace_relative_path: path,
                materialization_format: MaterializationFormat::named("other")
                    .unwrap_or_else(|error| panic!("format: {error}")),
                consumer: ConsumerId::new("other"),
                coordination: ConsumerCoordination::Uninterrupted,
            },
        ],
    );
    assert!(matches!(
        conflict,
        Err(DescriptorMergeError::IncompatibleSurfaceDeclarations { .. })
    ));
}

#[test]
fn filesystem_materializes_marker_and_detects_content_drift() {
    let workspace = TempDir::new().unwrap_or_else(|error| panic!("create Workspace: {error}"));
    let source = TempDir::new().unwrap_or_else(|error| panic!("create source: {error}"));
    let manifest = b"---\nname: review\ndescription: Reviews code\n---\nbody\n";
    fs::write(source.path().join("SKILL.md"), manifest)
        .unwrap_or_else(|error| panic!("write manifest: {error}"));
    fs::write(source.path().join("binary"), [0, 255, 1])
        .unwrap_or_else(|error| panic!("write binary: {error}"));
    let workspace_id = WorkspaceId::new("workspace");
    let path =
        SurfacePath::parse(".agents/skills").unwrap_or_else(|error| panic!("parse path: {error}"));
    let surface_key = SurfaceKey::for_workspace(&workspace_id, path.as_str());
    let adapter = FilesystemSurfaceAdapter::new(
        workspace_id,
        workspace.path().to_path_buf(),
        surface_key,
        path,
    );
    let selected = desired("review", "1", manifest);
    let snapshot = SourceSnapshot::borrowed(selected.1, source.path().to_path_buf());
    let operation_id = EffectOperationId::new("operation");
    let paths = OperationPaths::for_operation(&adapter.surface_root(), &operation_id);
    let fingerprint = adapter
        .stage(&snapshot, &ManagedIdentity::new("owned"), &paths)
        .unwrap_or_else(|error| panic!("stage package: {error}"));
    adapter
        .apply_create(&selected.0.name, &paths)
        .unwrap_or_else(|error| panic!("apply create: {error}"));

    let scan = adapter
        .scan()
        .unwrap_or_else(|error| panic!("scan surface: {error}"));
    assert_eq!(
        scan.targets,
        BTreeMap::from([(
            "review".to_string(),
            TargetObservation::Managed {
                marker_identity: ManagedIdentity::new("owned"),
                fingerprint: fingerprint.clone(),
            },
        )])
    );
    fs::write(
        adapter.surface_root().join("review").join("binary"),
        b"drift",
    )
    .unwrap_or_else(|error| panic!("write drift: {error}"));
    let drifted = adapter
        .scan()
        .unwrap_or_else(|error| panic!("scan drift: {error}"));
    assert_ne!(
        drifted.targets["review"],
        TargetObservation::Managed {
            marker_identity: ManagedIdentity::new("owned"),
            fingerprint,
        }
    );
}

#[test]
fn recovery_requires_manual_action_for_unknown_disk_state() {
    let workspace = TempDir::new().unwrap_or_else(|error| panic!("create Workspace: {error}"));
    let workspace_id = WorkspaceId::new("workspace");
    let path =
        SurfacePath::parse(".agents/skills").unwrap_or_else(|error| panic!("parse path: {error}"));
    let surface_key = SurfaceKey::for_workspace(&workspace_id, path.as_str());
    let adapter = FilesystemSurfaceAdapter::new(
        workspace_id.clone(),
        workspace.path().to_path_buf(),
        surface_key.clone(),
        path,
    );
    let root = adapter
        .ensure_surface_root()
        .unwrap_or_else(|error| panic!("create surface: {error}"));
    fs::create_dir(root.join("review")).unwrap_or_else(|error| panic!("create target: {error}"));
    fs::write(root.join("review").join("SKILL.md"), b"unknown")
        .unwrap_or_else(|error| panic!("write target: {error}"));
    let selected = desired("review", "1", b"planned");
    let operation = EffectOperation {
        operation_id: EffectOperationId::new("operation"),
        generation: Generation::new(1),
        workspace_id,
        surface_key,
        locator: "review".to_string(),
        target_name: selected.0.name,
        kind: EffectOperationKind::Create,
        phase: EffectOperationPhase::Prepared,
        previous_state: OperationState::Missing,
        planned_state: OperationState::Present(
            AppliedFingerprint::parse(fingerprint('a'))
                .unwrap_or_else(|error| panic!("parse fingerprint: {error}")),
        ),
        previous_identity: None,
        planned_identity: Some(ManagedIdentity::new("owned")),
        previous_managed: None,
        planned_desired: Some(selected.1),
        staging_path: root.join("staging"),
        backup_path: root.join("backup"),
    };
    assert_eq!(
        adapter
            .recovery_decision(&operation)
            .unwrap_or_else(|error| panic!("decide recovery: {error}")),
        RecoveryDecision::RecoveryRequired
    );
}
