//! Plugin operations use Node evidence instead of Substrate effects.
use super::*;
use pretty_assertions::assert_eq;

#[derive(Clone, Copy)]
pub(super) enum Outcome {
    Installed,
    ItemFailed,
    Interrupted,
}

/// Negotiates plugin installation explicitly; unsupported Nodes remain clone-capable.
pub(super) fn capabilities(node: &Node) -> Vec<NodeCapability> {
    let mut capabilities = vec![
        NodeCapability::RepositoryClone,
        NodeCapability::RuntimeControl,
    ];
    if node.plugin_capable.load(Ordering::SeqCst) {
        capabilities.push(NodeCapability::PluginInstall);
    }
    if node.agents.enabled.load(Ordering::SeqCst) {
        capabilities.push(NodeCapability::AgentSession);
    }
    if node.deliveries.enabled.load(Ordering::SeqCst) {
        capabilities.push(NodeCapability::RevisionDelivery);
    }
    if node.restores.enabled.load(Ordering::SeqCst) {
        capabilities.push(NodeCapability::RevisionRestore);
    }
    capabilities
}

/// Simulates a bounded Node result while preserving the actual controlled command identities.
pub(super) fn complete(
    node: &Node,
    envelope: &ControlledPlugins,
    identity: &NodeRuntimeIdentity,
) -> PluginsResultMessage {
    envelope.validate().unwrap();
    let PluginCommand::Install(input) = &envelope.command else {
        panic!("expected installation")
    };
    let outcome = *node.plugin_outcome.lock().unwrap();
    let payload = match outcome {
        Outcome::Interrupted => PluginExecutionResult::PluginsFailed(PluginsFailed {
            node: identity.clone(),
            failure: PluginsFailureCode::Interrupted,
        }),
        Outcome::Installed | Outcome::ItemFailed => {
            PluginExecutionResult::PluginsCompleted(PluginsCompleted {
                node: identity.clone(),
                items: input
                    .payload
                    .spec
                    .plugins
                    .iter()
                    .map(|p| PluginItemResult {
                        plugin_id: p.plugin_id.clone(),
                        outcome: match outcome {
                            Outcome::Installed => PluginItemOutcome::Installed {
                                version: p.version.clone(),
                            },
                            Outcome::ItemFailed => PluginItemOutcome::Failed {
                                failure: PluginFailureCode::ChecksumMismatch,
                            },
                            Outcome::Interrupted => unreachable!(),
                        },
                    })
                    .collect(),
            })
        }
    };
    PluginsResultMessage {
        protocol_version: CURRENT_PROTOCOL_VERSION,
        operation_id: input.operation_id.clone(),
        execution_id: input.execution_id.clone(),
        sequence: Sequence::new(/*value*/ 1),
        payload,
    }
}

/// Supplies an externally owned snapshot; the Controller must preserve every field.
fn input() -> proto::ExecutionInput {
    // Deserializing would bypass generated contract types; construct the actual input directly.
    proto::ExecutionInput {
        spec: Some(proto::execution_input::Spec::InstallPlugins(
            proto::InstallPluginsSpec {
                plugins: vec![proto::PluginInstall {
                    plugin_id: "official/ora-space.echo".into(),
                    version: "1.2.3".into(),
                    universal: Some(proto::PluginDownload {
                        url: "https://example.com/echo.orax".into(),
                        sha256: "a".repeat(64),
                    }),
                    targets: vec![],
                }],
            },
        )),
    }
}

/// Both successful and per-item failed evidence finish the step without Substrate plugin writes.
#[test]
fn plugin_step_preserves_input_and_advances_only_after_terminal_items() {
    for outcome in [Outcome::Installed, Outcome::ItemFailed] {
        scenario("main", |world| async move {
            world.cloud.plan_plugins(input());
            *world.node.plugin_outcome.lock().unwrap() = outcome;
            let operation = world.cloud.queue(proto::OperationKind::CreateWorkspace);
            settled(&world, &operation, proto::OperationState::Succeeded).await;
            let records = world.cloud.plugin_records();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0].input, Some(input()));
            assert!(matches!(
                records[0].result.as_ref().unwrap().outcome,
                Some(proto::execution_result::Outcome::PluginsResult(_))
            ));
            assert!(world.cloud.workspace().admission_open);
            let events = world.timeline.events();
            assert!(
                position(&events, &Event::Advanced { to: "plugin" })
                    < position(&events, &Event::Advanced { to: "done" })
            );
            assert!(!events.contains(&Event::Substrate {
                method: "PUT",
                kind: "unsupported"
            }));
        });
    }
}

/// A Node without installation capability never receives a plugin dispatch.
#[test]
fn missing_capability_blocks_without_opening_admission() {
    scenario("main", |world| async move {
        world.node.plugin_capable.store(false, Ordering::SeqCst);
        world.cloud.plan_plugins(input());
        let operation = world.cloud.queue(proto::OperationKind::CreateWorkspace);
        settled(&world, &operation, proto::OperationState::Blocked).await;
        assert!(world.cloud.plugin_records().is_empty());
        assert!(!world.cloud.workspace().admission_open);
    });
}

/// Whole-execution interruption remains retryable and cannot open Workspace admission.
#[test]
fn interrupted_plugin_execution_defers_the_operation() {
    scenario("main", |world| async move {
        *world.node.plugin_outcome.lock().unwrap() = Outcome::Interrupted;
        world.cloud.plan_plugins(input());
        let operation = world.cloud.queue(proto::OperationKind::CreateWorkspace);
        settled(&world, &operation, proto::OperationState::RetryWait).await;
        assert_eq!(world.cloud.plugin_records().len(), 1);
        assert!(!world.cloud.workspace().admission_open);
    });
}
