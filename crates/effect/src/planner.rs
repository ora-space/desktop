use crate::{
    AppliedFingerprint, Condition, ConditionReason, ConditionSubject, DesiredSkillState,
    Generation, ManagedIdentity, ManagedIdentityGenerator, ManagedSkill, SkillSelectionKey,
    SurfaceKey, SurfaceLifecycle,
};
use std::collections::{BTreeMap, BTreeSet};

/// Live disk state at one adapter-resolved target locator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetObservation {
    Missing,
    Preserved,
    Managed {
        marker_identity: ManagedIdentity,
        fingerprint: AppliedFingerprint,
    },
    Invalid {
        message: String,
    },
}

/// A safe per-locator transition selected by the pure planner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanOperationKind {
    Create {
        desired: DesiredSkillState,
        managed_identity: ManagedIdentity,
    },
    Update {
        previous: ManagedSkill,
        desired: DesiredSkillState,
    },
    AdvanceGeneration {
        previous: ManagedSkill,
    },
    Replace {
        previous: ManagedSkill,
        desired: DesiredSkillState,
        managed_identity: ManagedIdentity,
    },
    Delete {
        previous: ManagedSkill,
    },
}

/// One planned transition plus whether it needs a consumer-visible filesystem mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanOperation {
    pub locator: String,
    pub kind: PlanOperationKind,
    pub requires_filesystem_mutation: bool,
}

/// Complete plan for a surface scan; conflicts block only their own locators.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReconcilePlan {
    pub generation: Generation,
    pub operations: Vec<PlanOperation>,
    pub conditions: Vec<Condition>,
}

impl ReconcilePlan {
    /// Returns whether consumer quiescence is warranted by at least one safe disk mutation.
    pub fn has_filesystem_mutations(&self) -> bool {
        self.operations
            .iter()
            .any(|operation| operation.requires_filesystem_mutation)
    }

    /// Returns whether the complete desired generation is already represented on disk.
    pub fn is_current(&self) -> bool {
        self.operations.is_empty() && self.conditions.is_empty()
    }
}

/// Pure diff planner for one Workspace and physical Skill surface.
pub struct Planner<'a, IdentityGenerator> {
    identity_generator: &'a IdentityGenerator,
}

/// Groups one immutable surface snapshot so planner calls stay self-documenting.
pub struct PlannerInput<'a> {
    pub surface_key: &'a SurfaceKey,
    pub lifecycle: SurfaceLifecycle,
    pub generation: Generation,
    pub desired: &'a BTreeMap<SkillSelectionKey, DesiredSkillState>,
    pub managed: &'a [ManagedSkill],
    pub observed: &'a BTreeMap<String, TargetObservation>,
    pub occurred_at: i64,
}

impl<'a, IdentityGenerator> Planner<'a, IdentityGenerator>
where
    IdentityGenerator: ManagedIdentityGenerator,
{
    pub fn new(identity_generator: &'a IdentityGenerator) -> Self {
        Self { identity_generator }
    }

    /// Computes the full diff before any mutation and preserves safe work beside conflicts.
    pub fn plan(&self, input: PlannerInput<'_>) -> ReconcilePlan {
        let PlannerInput {
            surface_key,
            lifecycle,
            generation,
            desired,
            managed,
            observed,
            occurred_at,
        } = input;
        let mut desired_by_locator: BTreeMap<
            String,
            Vec<(&SkillSelectionKey, &DesiredSkillState)>,
        > = BTreeMap::new();
        if lifecycle == SurfaceLifecycle::Active {
            for (selection, state) in desired {
                desired_by_locator
                    .entry(state.state().name.canonical().to_string())
                    .or_default()
                    .push((selection, state));
            }
        }
        let mut managed_by_locator: BTreeMap<String, Vec<&ManagedSkill>> = BTreeMap::new();
        for ledger in managed {
            managed_by_locator
                .entry(ledger.locator.clone())
                .or_default()
                .push(ledger);
        }
        let locators = desired_by_locator
            .keys()
            .chain(managed_by_locator.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut operations = Vec::new();
        let mut conditions = Vec::new();

        for locator in locators {
            let desired_here = desired_by_locator
                .get(&locator)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let managed_here = managed_by_locator
                .get(&locator)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let observation = observed
                .get(&locator)
                .cloned()
                .unwrap_or(TargetObservation::Missing);

            if desired_here.len() > 1 {
                for (selection_key, _) in desired_here {
                    conditions.push(Condition::new(
                        ConditionSubject::DesiredSkill {
                            selection_key: (*selection_key).clone(),
                        },
                        ConditionReason::DesiredCollision,
                        "multiple desired Skills resolve to the same surface locator",
                        occurred_at,
                        generation,
                    ));
                }
                continue;
            }
            if managed_here.len() > 1 {
                conditions.push(Condition::new(
                    ConditionSubject::Surface {
                        surface_key: surface_key.clone(),
                    },
                    ConditionReason::OwnershipConflict,
                    "multiple ownership ledgers claim one surface locator",
                    occurred_at,
                    generation,
                ));
                continue;
            }

            match (desired_here.first(), managed_here.first()) {
                (Some((selection_key, desired)), None) => match observation {
                    TargetObservation::Missing => operations.push(PlanOperation {
                        locator,
                        kind: PlanOperationKind::Create {
                            desired: (*desired).clone(),
                            managed_identity: self.identity_generator.generate_managed_identity(),
                        },
                        requires_filesystem_mutation: true,
                    }),
                    TargetObservation::Preserved => conditions.push(Condition::new(
                        ConditionSubject::DesiredSkill {
                            selection_key: (*selection_key).clone(),
                        },
                        ConditionReason::PreservedConflict,
                        "an unowned Skill already occupies the target locator",
                        occurred_at,
                        generation,
                    )),
                    TargetObservation::Managed { .. } => conditions.push(Condition::new(
                        ConditionSubject::DesiredSkill {
                            selection_key: (*selection_key).clone(),
                        },
                        ConditionReason::OwnershipConflict,
                        "a marker without a matching ledger cannot grant ownership",
                        occurred_at,
                        generation,
                    )),
                    TargetObservation::Invalid { message } => conditions.push(Condition::new(
                        ConditionSubject::DesiredSkill {
                            selection_key: (*selection_key).clone(),
                        },
                        ConditionReason::PreservedConflict,
                        message,
                        occurred_at,
                        generation,
                    )),
                },
                (None, Some(previous)) => {
                    if let Some(requires_filesystem_mutation) = validate_owned_observation(
                        previous,
                        &observation,
                        generation,
                        occurred_at,
                        &mut conditions,
                    ) {
                        operations.push(PlanOperation {
                            locator,
                            kind: PlanOperationKind::Delete {
                                previous: (*previous).clone(),
                            },
                            requires_filesystem_mutation,
                        });
                    }
                }
                (Some((selection_key, desired)), Some(previous)) => {
                    if let Some(requires_filesystem_mutation) = validate_owned_observation(
                        previous,
                        &observation,
                        generation,
                        occurred_at,
                        &mut conditions,
                    ) {
                        if &previous.selection_key == *selection_key {
                            if previous.state == **desired
                                && !matches!(observation, TargetObservation::Missing)
                            {
                                if previous.applied_generation == generation {
                                    continue;
                                }
                                operations.push(PlanOperation {
                                    locator,
                                    kind: PlanOperationKind::AdvanceGeneration {
                                        previous: (*previous).clone(),
                                    },
                                    requires_filesystem_mutation: false,
                                });
                                continue;
                            }
                            operations.push(PlanOperation {
                                locator,
                                kind: PlanOperationKind::Update {
                                    previous: (*previous).clone(),
                                    desired: (*desired).clone(),
                                },
                                // A missing directory must be rebuilt even if state is unchanged.
                                requires_filesystem_mutation: true,
                            });
                        } else {
                            operations.push(PlanOperation {
                                locator,
                                kind: PlanOperationKind::Replace {
                                    previous: (*previous).clone(),
                                    desired: (*desired).clone(),
                                    managed_identity: self
                                        .identity_generator
                                        .generate_managed_identity(),
                                },
                                requires_filesystem_mutation: true,
                            });
                        }
                        let _ = requires_filesystem_mutation;
                    }
                }
                (None, None) => {}
            }
        }

        ReconcilePlan {
            generation,
            operations,
            conditions,
        }
    }
}

/// Proves the database ledger still owns its target and returns whether a delete needs disk I/O.
fn validate_owned_observation(
    managed: &ManagedSkill,
    observation: &TargetObservation,
    generation: Generation,
    occurred_at: i64,
    conditions: &mut Vec<Condition>,
) -> Option<bool> {
    match observation {
        TargetObservation::Missing => Some(false),
        TargetObservation::Managed {
            marker_identity,
            fingerprint,
        } if marker_identity != &managed.managed_identity => {
            conditions.push(Condition::new(
                ConditionSubject::ManagedSkill {
                    managed_identity: managed.managed_identity.clone(),
                },
                ConditionReason::OwnershipConflict,
                "the ownership marker does not match the database ledger",
                occurred_at,
                generation,
            ));
            None
        }
        TargetObservation::Managed { fingerprint, .. }
            if fingerprint != &managed.applied_fingerprint =>
        {
            conditions.push(Condition::new(
                ConditionSubject::ManagedSkill {
                    managed_identity: managed.managed_identity.clone(),
                },
                ConditionReason::DriftConflict,
                "managed content differs from the last applied fingerprint",
                occurred_at,
                generation,
            ));
            None
        }
        TargetObservation::Managed { .. } => Some(true),
        TargetObservation::Preserved => {
            conditions.push(Condition::new(
                ConditionSubject::ManagedSkill {
                    managed_identity: managed.managed_identity.clone(),
                },
                ConditionReason::OwnershipConflict,
                "the managed locator no longer has a valid ownership marker",
                occurred_at,
                generation,
            ));
            None
        }
        TargetObservation::Invalid { message } => {
            conditions.push(Condition::new(
                ConditionSubject::ManagedSkill {
                    managed_identity: managed.managed_identity.clone(),
                },
                ConditionReason::OwnershipConflict,
                message.clone(),
                occurred_at,
                generation,
            ));
            None
        }
    }
}
