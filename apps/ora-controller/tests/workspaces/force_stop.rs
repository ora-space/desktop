//! Force-stop delivery exercises arbitrary Cloud plans, independently of the caller's role.
use super::*;
use ora_controller_proto::v1::effect_request::Request as EffectRequest;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;

/// Creates one exact Cloud target, including targets whose ensure never executed.
fn terminate(id: &str, sandbox: &str) -> proto::Effect {
    proto::Effect {
        id: id.into(),
        kind: proto::EffectKind::SandboxTerminate as i32,
        state: proto::EffectState::Planned as i32,
        request: Some(proto::EffectRequest {
            request: Some(EffectRequest::SandboxTerminate(
                proto::SandboxTerminateRequest {
                    project_id: workspace_cloud::PROJECT.into(),
                    workspace_id: WORKSPACE.into(),
                    sandbox_instance_id: sandbox.into(),
                },
            )),
        }),
        ..proto::Effect::default()
    }
}

/// Builds the RPC's generic intention without assuming a tenant-admin or operator producer.
fn plan(id: &str, effects: Vec<proto::Effect>) -> proto::RuntimeForceStopPlan {
    proto::RuntimeForceStopPlan {
        id: id.into(),
        version: 7,
        workspace_id: WORKSPACE.into(),
        effects,
    }
}

/// Supplies an exact Substrate journal result; missing and false fields remain untouched.
fn reply(result: Value) -> (StatusCode, Value) {
    (StatusCode::OK, json!({"state":"succeeded","result":result}))
}

/// Waits for a later ordinary scheduler wake without using an arbitrary sleep.
async fn next_wake(world: &World) {
    let before = world
        .timeline
        .events()
        .iter()
        .filter(|event| **event == Event::ForceListed)
        .count();
    world
        .timeline
        .until(|events| {
            events
                .iter()
                .filter(|event| **event == Event::ForceListed)
                .count()
                > before
        })
        .await;
}

/// Every provided target retains its identity; false/missing evidence and a rejected peer do not
/// confirm, starve later independent targets, or make an unexecuted ensure disappear.
#[test]
fn force_stop_preserves_all_cloud_effect_targets_and_evidence() {
    scenario("main", |world| async move {
        let targets = [
            (
                "false",
                "sandbox-false",
                reply(json!({"terminated":false,"lateEnsureFenced":true})),
            ),
            ("missing", "sandbox-missing", reply(json!({}))),
            (
                "late-false",
                "sandbox-late-false",
                reply(json!({"terminated":true,"lateEnsureFenced":false})),
            ),
            (
                "reject",
                "sandbox-reject",
                (StatusCode::CONFLICT, json!({"error":"request_conflict"})),
            ),
            (
                "never-ensured",
                "planned-ensure-with-no-row",
                reply(json!({"terminated":true,"lateEnsureFenced":true})),
            ),
            (
                "next-confirmed",
                "second-independent-sandbox",
                reply(json!({"terminated":true,"lateEnsureFenced":true})),
            ),
            (
                "operator-target",
                "operator-supplied-sandbox",
                reply(json!({"terminated":true,"lateEnsureFenced":true})),
            ),
        ];
        for (id, _, response) in &targets {
            world
                .substrate
                .replies
                .lock()
                .unwrap()
                .by_id
                .insert((*id).into(), response.clone());
        }
        world.cloud.force_plans(
            vec![
                plan(
                    "arbitrary-reclaim-intention",
                    targets[..6]
                        .iter()
                        .map(|(id, sandbox, _)| terminate(id, sandbox))
                        .collect(),
                ),
                plan(
                    "operator-intention",
                    vec![terminate(targets[6].0, targets[6].1)],
                ),
            ],
            HashMap::new(),
        );
        world
            .timeline
            .until(|_| world.cloud.force_calls().1.len() == 3)
            .await;
        let (permits, confirmations) = world.cloud.force_calls();
        let actual: BTreeMap<_, _> = world
            .substrate
            .replies
            .lock()
            .unwrap()
            .puts
            .iter()
            .map(|(id, body)| {
                (
                    id.clone(),
                    (
                        body["sandboxInstanceId"].as_str().unwrap().to_owned(),
                        body["authorization"]["sandboxId"]
                            .as_str()
                            .unwrap()
                            .to_owned(),
                    ),
                )
            })
            .collect();
        let expected: BTreeMap<_, _> = targets
            .iter()
            .map(|(id, sandbox, _)| ((*id).into(), ((*sandbox).into(), (*sandbox).into())))
            .collect();
        assert_eq!(actual, expected);
        let requests: BTreeMap<_, _> = permits
            .into_iter()
            .map(|request| (request.effect_id, (request.epoch, request.force_stop_id)))
            .collect();
        assert_eq!(
            requests,
            targets
                .iter()
                .enumerate()
                .map(|(index, (id, _, _))| (
                    (*id).into(),
                    (
                        workspace_cloud::EPOCH,
                        if index < 6 {
                            "arbitrary-reclaim-intention"
                        } else {
                            "operator-intention"
                        }
                        .into()
                    )
                ))
                .collect()
        );
        assert_eq!(
            confirmations
                .into_iter()
                .map(|confirmation| (
                    confirmation.force_stop_id,
                    confirmation.version,
                    confirmation.effect_id,
                    confirmation.terminated,
                    confirmation.late_ensure_fenced
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "arbitrary-reclaim-intention".into(),
                    7,
                    "never-ensured".into(),
                    true,
                    true
                ),
                (
                    "arbitrary-reclaim-intention".into(),
                    8,
                    "next-confirmed".into(),
                    true,
                    true
                ),
                (
                    "operator-intention".into(),
                    7,
                    "operator-target".into(),
                    true,
                    true
                ),
            ]
        );
    });
}

/// Force-stop owns a separate task, so a held ordinary effect read cannot prevent termination.
#[test]
fn force_stop_progresses_while_an_ordinary_operation_is_waiting() {
    scenario("main", |world| async move {
        let (arrived, read_started) = oneshot::channel();
        let (release, held) = oneshot::channel();
        world.substrate.replies.lock().unwrap().get_barrier = Some((arrived, held));
        let ordinary = world.cloud.queue(proto::OperationKind::CreateWorkspace);
        let ordinary_effect = tokio::time::timeout(Duration::from_secs(/*secs*/ 10), read_started)
            .await
            .unwrap()
            .unwrap();
        world.substrate.replies.lock().unwrap().by_id.insert(
            "independent-force".into(),
            reply(json!({"terminated":true,"lateEnsureFenced":true})),
        );
        world.cloud.force_plans(
            vec![plan(
                "force-during-ordinary-wait",
                vec![terminate("independent-force", "independent-sandbox")],
            )],
            HashMap::new(),
        );
        world
            .timeline
            .until(|_| world.cloud.force_calls().1.len() == 1)
            .await;

        // The held ordinary RPC has not been released: only the independent task can finish here.
        assert_eq!(
            world.cloud.operation(&ordinary).state(),
            proto::OperationState::Running
        );
        assert_eq!(
            world
                .cloud
                .force_calls()
                .1
                .into_iter()
                .map(|confirmation| (
                    confirmation.force_stop_id,
                    confirmation.effect_id,
                    confirmation.terminated,
                    confirmation.late_ensure_fenced,
                ))
                .collect::<Vec<_>>(),
            vec![(
                "force-during-ordinary-wait".into(),
                "independent-force".into(),
                true,
                true
            )],
        );
        assert!(
            world
                .substrate
                .replies
                .lock()
                .unwrap()
                .puts
                .iter()
                .all(|(id, _)| id != &ordinary_effect)
        );
        release.send(()).unwrap();
        settled(&world, &ordinary, proto::OperationState::Succeeded).await;
    });
}

/// Shutdown is attempted before a new terminate PUT, but even a missing close acknowledgement
/// cannot become a terminal-result prerequisite or a substitute for Substrate evidence.
#[test]
fn force_stop_attempts_bounded_control_close_before_terminate() {
    for ignore_close in [false, true] {
        scenario("main", |world| async move {
            let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
            settled(&world, &create, proto::OperationState::Succeeded).await;
            let sandbox = world
                .cloud
                .effect_identity()
                .0
                .into_iter()
                .find(|effect| effect.kind() == proto::EffectKind::SandboxEnsure)
                .unwrap()
                .id;
            world
                .node
                .ignore_close
                .store(ignore_close, Ordering::SeqCst);
            world.substrate.replies.lock().unwrap().by_id.insert(
                "stop".into(),
                reply(json!({"terminated":true,"lateEnsureFenced":true})),
            );
            let before = world.timeline.events().len();
            world.cloud.force_plans(
                vec![plan("close-intention", vec![terminate("stop", &sandbox)])],
                HashMap::new(),
            );
            world
                .timeline
                .until(|_| world.cloud.force_calls().1.len() == 1)
                .await;
            let events = world.timeline.events()[before..].to_vec();
            assert!(
                position(&events, &Event::ShutdownClose)
                    < position(
                        &events,
                        &Event::Substrate {
                            method: "PUT",
                            kind: "sandbox_terminate"
                        }
                    )
            );
            assert_eq!(
                world
                    .cloud
                    .force_calls()
                    .1
                    .into_iter()
                    .map(|confirmation| (
                        confirmation.effect_id,
                        confirmation.terminated,
                        confirmation.late_ensure_fenced
                    ))
                    .collect::<Vec<_>>(),
                vec![("stop".into(), true, true)]
            );
            assert!(
                !events
                    .iter()
                    .any(|event| matches!(event, Event::CommandReceived { .. })),
                "control close must not synthesize EndSession"
            );
        });
    }
}

/// Both actual server tombstone response shapes block without replacing an ensure identity;
/// recovery GET of the failed journal cannot issue a new PUT either.
#[test]
fn tombstoned_ensure_does_not_retry_or_change_generation() {
    for admission in [false, true] {
        scenario("main", |world| async move {
            let failure = if admission {
                json!({"error":"sandbox_terminated"})
            } else {
                json!({"state":"failed","result":{},"error":"sandbox_terminated"})
            };
            world.substrate.replies.lock().unwrap().ensure =
                Some((StatusCode::SERVICE_UNAVAILABLE, failure));
            let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
            settled(&world, &create, proto::OperationState::Blocked).await;
            let identity = world.cloud.effect_identity();
            let puts = world.substrate.replies.lock().unwrap().puts.clone();
            assert_eq!(puts.len(), 1);
            world.cloud.reoffer_retries();
            next_wake(&world).await;
            assert_eq!(
                (
                    world.cloud.operation(&create).state(),
                    world.cloud.effect_identity(),
                    world.substrate.replies.lock().unwrap().puts.clone()
                ),
                (proto::OperationState::Blocked, identity, puts)
            );
        });
    }
    scenario("main", |world| async move {
        world.substrate.replies.lock().unwrap().ensure = Some((
            StatusCode::SERVICE_UNAVAILABLE,
            json!({"state":"failed","result":{},"error":"docker_failure"}),
        ));
        let create = world.cloud.queue(proto::OperationKind::CreateWorkspace);
        settled(&world, &create, proto::OperationState::RetryWait).await;
        let identity = world.cloud.effect_identity();
        let effect = &identity.0[0];
        let old = world
            .substrate
            .journal
            .lock()
            .unwrap()
            .get(&effect.id)
            .unwrap()
            .clone();
        let mut tombstone = old;
        tombstone["error"] = json!("sandbox_terminated");
        world
            .substrate
            .journal
            .lock()
            .unwrap()
            .insert(effect.id.clone(), tombstone);
        let puts = world.substrate.replies.lock().unwrap().puts.clone();
        world.cloud.reoffer_retries();
        settled(&world, &create, proto::OperationState::Blocked).await;
        assert_eq!(
            (
                world.cloud.effect_identity(),
                world.substrate.replies.lock().unwrap().puts.clone()
            ),
            (identity, puts)
        );
    });
}

/// Negative transport/authority facts never confirm termination; an unreachable Node still does
/// not prevent genuine dual Substrate evidence from being reported.
#[test]
fn absence_and_unavailable_nodes_are_not_termination_confirmation() {
    for outcome in [
        "404",
        "unreachable",
        "cloud_unavailable",
        "cloud_unavailable_delete",
        "lease_lost",
        "valid",
    ] {
        scenario("main", |world| async move {
            let mut permits = HashMap::new();
            let response = match outcome {
                "404" => (StatusCode::NOT_FOUND, json!({"error":"not_found"})),
                "unreachable" => (
                    StatusCode::BAD_GATEWAY,
                    json!({"error":"upstream_unreachable"}),
                ),
                "cloud_unavailable" | "cloud_unavailable_delete" => {
                    permits.insert(
                        "first".into(),
                        Err(tonic::Status::unavailable("cloud_unavailable")),
                    );
                    reply(json!({"terminated":true,"lateEnsureFenced":true}))
                }
                "lease_lost" => {
                    permits.insert(
                        "first".into(),
                        Err(tonic::Status::failed_precondition("stale_controller")),
                    );
                    reply(json!({"terminated":true,"lateEnsureFenced":true}))
                }
                "valid" => reply(json!({"terminated":true,"lateEnsureFenced":true})),
                _ => panic!("unknown outcome"),
            };
            world
                .substrate
                .replies
                .lock()
                .unwrap()
                .by_id
                .insert("first".into(), response);
            if outcome == "unreachable" {
                world
                    .substrate
                    .replies
                    .lock()
                    .unwrap()
                    .unanswered
                    .insert("first".into());
            }
            world.substrate.replies.lock().unwrap().by_id.insert(
                "second".into(),
                reply(json!({"terminated":true,"lateEnsureFenced":true})),
            );
            let mut first = terminate("first", "node-never-connected");
            if outcome == "cloud_unavailable_delete" {
                first.kind = proto::EffectKind::WorkspaceDataDelete as i32;
                first.request = Some(proto::EffectRequest {
                    request: Some(EffectRequest::WorkspaceDataDelete(
                        proto::WorkspaceDataDeleteRequest {
                            project_id: workspace_cloud::PROJECT.into(),
                            workspace_id: WORKSPACE.into(),
                        },
                    )),
                });
            }
            world.cloud.force_plans(
                vec![plan(
                    "unreachable-node-intention",
                    vec![first, terminate("second", "another-node-never-connected")],
                )],
                permits,
            );
            if outcome == "valid" {
                world
                    .timeline
                    .until(|_| world.cloud.force_calls().1.len() == 2)
                    .await;
            } else if matches!(
                outcome,
                "cloud_unavailable" | "cloud_unavailable_delete" | "lease_lost"
            ) {
                world
                    .timeline
                    .until(|_| !world.cloud.force_calls().0.is_empty())
                    .await;
                // Shutdown waits for the in-flight request's actual return; no timer is evidence.
                world.controller.stop().await;
                let (requests, confirmations) = world.cloud.force_calls();
                assert_eq!(
                    (
                        requests
                            .into_iter()
                            .map(|request| request.effect_id)
                            .collect::<Vec<_>>(),
                        confirmations,
                        world.substrate.replies.lock().unwrap().puts.clone()
                    ),
                    (vec!["first".into()], Vec::new(), Vec::new())
                );
                world.controller.start().await;
                return;
            } else {
                world
                    .timeline
                    .until(|_| world.cloud.force_calls().1.len() == 1)
                    .await;
            }
            let confirmations = world
                .cloud
                .force_calls()
                .1
                .into_iter()
                .map(|confirmation| {
                    (
                        confirmation.effect_id,
                        confirmation.terminated,
                        confirmation.late_ensure_fenced,
                    )
                })
                .collect::<Vec<_>>();
            let expected = if outcome == "valid" {
                vec![("first".into(), true, true), ("second".into(), true, true)]
            } else {
                vec![("second".into(), true, true)]
            };
            assert_eq!(confirmations, expected);
            assert!(!world.timeline.events().contains(&Event::Registered {
                incarnation: "incarnation-1".into()
            }));
        });
    }
}
