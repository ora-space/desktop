#![allow(clippy::unwrap_used)]

use super::*;
use crate::model_proxy::{ModelAccess, ModelAccessStatus, tests::Gateway};
use crate::session::ports::{QueuedCommand, SessionCommand};
use ora_node_protocol::{AgentSessionEnded, CommandId, ModelBindingId, Sequence, ThreadEvent};
use pretty_assertions::assert_eq;
#[path = "../../../tests/support/session_stops.rs"]
mod stops;
use std::sync::{Arc, Mutex, atomic::Ordering};

/// The ending path must return before plugin discovery or process launch.
struct CheckoutPath(std::path::PathBuf);
impl crate::PriorRevisionRestore for CheckoutPath {
    /// Early-ending sessions have no prior input and must never reach restore work.
    async fn restore(
        &self,
        _request: crate::RestoreRequest,
        _cancel: crate::RestoreCancel,
    ) -> Result<crate::Restored, crate::RestoreFailure> {
        panic!("an unstarted model session must not restore a prior Revision")
    }
}
impl crate::CheckoutResolver for CheckoutPath {
    /// Resolves the isolated checkout without invoking Git for an unstarted session.
    fn checkout(&self, _execution: &ExecutionId) -> Option<ora_node_db::ClonedCheckout> {
        let commit = "0123456789abcdef0123456789abcdef01234567";
        Some(ora_node_db::ClonedCheckout {
            path: self.0.clone(),
            spec: ora_node_protocol::CloneExecutionSpec {
                node_id: ora_node_protocol::NodeId::new("node"),
                repository_url: ora_node_protocol::GitRepositoryUrl::parse(
                    "https://git.ora.test/tenant/project.git",
                )
                .ok()?,
                snapshot_ref: ora_node_protocol::SnapshotRef::parse(&format!(
                    "refs/ora/upstreams/9d8c7b6a-5f4e-4d3c-8b2a-1f0e9d8c7b6a/snapshots/{commit}"
                ))
                .ok()?,
                base_commit: ora_node_protocol::CommitId::new(commit),
                workspace_ref: ora_node_protocol::WorkspaceRef::parse(
                    "refs/heads/workspaces/4c3b2a19-0f8e-4d7c-9b6a-5f4e3d2c1b0a/main",
                )
                .ok()?,
            },
        })
    }
}
impl crate::PluginCatalog for CheckoutPath {
    type Lease = ();
    /// The no-launch branch holds and releases an empty catalog lease.
    fn lease(&self, _id: &ora_node_protocol::PluginId) {}
    /// Supplies a candidate path so grant denial is checked before any plugin discovery.
    fn installed(
        &self,
        _id: &ora_node_protocol::PluginId,
        _version: &ora_node_protocol::PluginVersion,
    ) -> Option<std::path::PathBuf> {
        Some(self.0.clone())
    }
    fn check_configuration(
        &self,
        _configuration: &ora_node_protocol::FrozenAgentConfiguration,
    ) -> Result<(), crate::InstalledConfigurationError> {
        Ok(())
    }
}

#[derive(Default)]
struct State {
    queued: Vec<QueuedCommand>,
    settled: Vec<(CommandId, CommandSettlement)>,
    ended: Option<AgentSessionEnded>,
    deferred: Option<AgentSessionEnded>,
}

#[derive(Clone, Default)]
struct Ledger {
    state: Arc<Mutex<State>>,
    stops: stops::Stops,
    read: Arc<Notify>,
    terminal: Arc<Notify>,
    deferred: Arc<Notify>,
}

impl SessionLedger for Ledger {
    type Error = std::io::Error;
    fn fail_session_turn(
        &self,
        _execution: &ExecutionId,
        _failure: ora_node_protocol::SessionTurnFailed,
    ) -> Result<(), Self::Error> {
        Err(std::io::Error::other(
            "model fixture has no lifecycle turns",
        ))
    }
    fn record_session_stop(
        &self,
        identity: &ora_node_db::SessionStopIdentity,
        at_ms: i64,
    ) -> Result<(), Self::Error> {
        self.stops.record(identity, at_ms)
    }
    fn session_stop_records(
        &self,
        execution: Option<&ExecutionId>,
    ) -> Result<Vec<ora_node_db::SessionStopRecord>, Self::Error> {
        self.stops.records(execution)
    }
    fn change_session_stop(
        &self,
        attempt: &ora_node_db::AttemptId,
        change: ora_node_db::SessionStopChange,
    ) -> Result<(), Self::Error> {
        self.stops.change(attempt, change)
    }
    fn defer_session_stop_end(
        &self,
        execution: &ExecutionId,
        ended: &AgentSessionEnded,
    ) -> Result<(), Self::Error> {
        self.stops.defer(execution, ended)
    }

    /// Deferred stop evidence has not sealed the session, even after its actor is gone.
    fn session_completed(&self, _execution: &ExecutionId) -> Result<bool, Self::Error> {
        Ok(self.state.lock().unwrap().ended.is_some())
    }

    /// The terminal intention must become durable before the actor transfers responsibility.
    fn defer_restore_end(
        &self,
        _execution: &ExecutionId,
        ended: &AgentSessionEnded,
    ) -> Result<(), Self::Error> {
        self.state.lock().unwrap().deferred = Some(ended.clone());
        self.deferred.notify_one();
        Ok(())
    }

    /// Reconciliation must not fabricate transcript records or terminal events.
    fn append_thread_event(
        &self,
        _execution: &ExecutionId,
        _event: ThreadEvent,
    ) -> Result<Sequence, Self::Error> {
        Err(std::io::Error::other("unexpected transcript write"))
    }

    /// The normal driver, after reconciliation, remains the sole terminal writer.
    fn end_session(
        &self,
        _execution: &ExecutionId,
        ended: AgentSessionEnded,
    ) -> Result<Sequence, Self::Error> {
        self.state.lock().unwrap().ended = Some(ended);
        self.terminal.notify_one();
        Ok(Sequence::new(/*value*/ 1))
    }

    /// Notifies tests only after the actor actually read the durable queue.
    fn queued_commands(&self, _execution: &ExecutionId) -> Result<Vec<QueuedCommand>, Self::Error> {
        let state = self.state.lock().unwrap();
        let queued = state
            .queued
            .iter()
            .filter(|command| {
                !state
                    .settled
                    .iter()
                    .any(|(settled, _)| *settled == command.command_id)
            })
            .cloned()
            .collect();
        self.read.notify_one();
        Ok(queued)
    }

    /// Captures the exact once-only durable settlement.
    fn settle_command(
        &self,
        _execution: &ExecutionId,
        command: &CommandId,
        settlement: CommandSettlement,
    ) -> Result<(), Self::Error> {
        self.state
            .lock()
            .unwrap()
            .settled
            .push((command.clone(), settlement));
        Ok(())
    }
}

/// Runs production reconciliation before delivering its already durable end, without a timer guess.
async fn delayed_end(
    ledger: Ledger,
    execution: ExecutionId,
    reason: EndSessionReason,
) -> EndSessionReason {
    let wake = Arc::new(Notify::new());
    let (_stop, stopping) = watch::channel(/*init*/ false);
    let actor_ledger = ledger.clone();
    let actor_wake = wake.clone();
    let actor = tokio::spawn(async move {
        reconcile(
            &actor_ledger,
            &execution,
            &actor_wake,
            stopping,
            END_RECONCILE_WAIT,
        )
        .await
    });
    ledger.read.notified().await;
    ledger.state.lock().unwrap().queued.push(QueuedCommand {
        command_id: CommandId::new("end-1"),
        command: SessionCommand::EndSession(reason),
    });
    wake.notify_one();
    actor.await.unwrap().unwrap()
}

#[tokio::test]
async fn initial_grant_ending_preserves_delayed_user_end_and_seals_empty_history() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Gateway::new(root.path());
    gateway
        .behavior
        .ending_initially
        .store(/*val*/ true, Ordering::SeqCst);
    let execution = ExecutionId::new("8b0e5a52-6f1c-4c55-9d3e-2a7b1f0c9e41");
    let grant = ModelAccess::open(
        gateway.config.clone(),
        &ModelBindingId::new("binding-1"),
        &execution,
        root.path(),
        crate::model_proxy::ModelRoute::LegacyOpenCode,
    )
    .await;
    let Err(error) = grant else {
        panic!("ending must not issue model authority")
    };
    assert_eq!(error.0, "model_session_ending");
    let ledger = Ledger::default();
    assert_eq!(
        delayed_end(
            ledger.clone(),
            execution.clone(),
            EndSessionReason::UserEnded
        )
        .await,
        EndSessionReason::UserEnded
    );
    let histories = root.path().join("sessions");
    seal_unstarted_history(&histories, &execution).unwrap();
    let history = ora_history::history_path(&histories, execution.as_str()).unwrap();
    assert_eq!(std::fs::read(history).unwrap(), Vec::<u8>::new());
    assert_eq!(
        ledger.state.lock().unwrap().settled,
        vec![(CommandId::new("end-1"), CommandSettlement::Executed)]
    );
    assert_eq!(gateway.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn production_driver_reconciles_early_user_end_without_launching_a_plugin() {
    use crate::{AgentSessions, SessionConfig, SessionHost};
    use ora_node_protocol::{
        AgentSessionEndReason, AgentSessionSpec, ContentBlock, GitIdentity, NodeId,
        NodeIncarnationId, NodeRuntimeIdentity, PluginId, PluginVersion, TurnId, UserTurn,
    };
    let root = tempfile::tempdir().unwrap();
    let gateway = Gateway::new(root.path());
    gateway
        .behavior
        .ending_initially
        .store(/*val*/ true, Ordering::SeqCst);
    let execution = ExecutionId::new("8b0e5a52-6f1c-4c55-9d3e-2a7b1f0c9e41");
    let ledger = Ledger::default();
    let node = NodeRuntimeIdentity {
        node_id: NodeId::new("node-1"),
        incarnation_id: NodeIncarnationId::new("incarnation-1"),
    };
    let sessions = AgentSessions::new(
        SessionConfig {
            stop: Default::default(),
            home_directory: root.path().into(),
            deno_path: root.path().join("must-not-be-executed"),
            timezone: chrono_tz::UTC,
            agent_ready_timeout: Duration::from_secs(/*secs*/ 1),
            model_proxy: Some(gateway.config.clone()),
            workload: crate::SessionWorkload::Shared,
        },
        node.clone(),
        ledger.clone(),
        CheckoutPath(root.path().into()),
        CheckoutPath(root.path().into()),
        CheckoutPath(root.path().into()),
    );
    sessions.start(
        ora_node_protocol::OperationId::new("operation-1"),
        execution.clone(),
        AgentSessionSpec {
            host_mcp_servers: Vec::new(),
            mcp_lifecycle: false,
            mcp: Vec::new(),
            agent_configuration: None,
            node_id: node.node_id.clone(),
            agent_plugin_id: PluginId::new("official/ora-space.opencode"),
            agent_plugin_version: PluginVersion::new("1.0.0"),
            checkout_execution_id: ExecutionId::new("clone-1"),
            model_binding_id: Some(ModelBindingId::new("binding-1")),
            prior_revision: None,
            workspace_directory: Vec::new(),
            git_identity: GitIdentity {
                name: "User".into(),
                email: "user@example.com".into(),
            },
            initial_turn: UserTurn {
                turn_id: TurnId::new("turn-1"),
                content: vec![ContentBlock::Text {
                    text: "Must not be sent".into(),
                }],
            },
            trigger: None,
        },
    );
    ledger.read.notified().await;
    ledger.state.lock().unwrap().queued.push(QueuedCommand {
        command_id: CommandId::new("end-1"),
        command: SessionCommand::EndSession(EndSessionReason::UserEnded),
    });
    sessions.command_arrived(&execution);
    ledger.terminal.notified().await;
    sessions.shutdown().await;
    assert_eq!(
        ledger.state.lock().unwrap().ended,
        Some(AgentSessionEnded {
            node,
            reason: AgentSessionEndReason::UserEnded,
            detail: None,
            plugin_id: None,
            setting_id: None,
            transport: None,
            scheduled_result: None,
            artifact_manifest: None,
        })
    );
    assert_eq!(
        std::fs::read(sessions.sealed_history(&execution).unwrap()).unwrap(),
        Vec::<u8>::new()
    );
    assert_eq!(gateway.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn renewal_ending_revokes_before_reconciling_exact_cancelled_reason() {
    let root = tempfile::tempdir().unwrap();
    let gateway = Gateway::new(root.path());
    gateway
        .behavior
        .ending_at_renewal
        .store(/*val*/ true, Ordering::SeqCst);
    let execution = ExecutionId::new("8b0e5a52-6f1c-4c55-9d3e-2a7b1f0c9e41");
    let mut access = ModelAccess::open(
        gateway.config.clone(),
        &ModelBindingId::new("binding-1"),
        &execution,
        root.path(),
        crate::model_proxy::ModelRoute::LegacyOpenCode,
    )
    .await
    .unwrap();
    let status = tokio::time::timeout(
        Duration::from_secs(/*secs*/ 4),
        access
            .status
            .wait_for(|status| *status != ModelAccessStatus::Active),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(*status, ModelAccessStatus::Ending);
    drop(status);
    access.close().await;
    let ledger = Ledger::default();
    assert_eq!(
        delayed_end(ledger.clone(), execution, EndSessionReason::Cancelled).await,
        EndSessionReason::Cancelled
    );
    assert_eq!(
        ledger.state.lock().unwrap().settled,
        vec![(CommandId::new("end-1"), CommandSettlement::Executed)]
    );
    assert_eq!(gateway.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn missing_durable_end_and_node_shutdown_have_bounded_outcomes() {
    let ledger = Ledger::default();
    let (stop, stopping) = watch::channel(/*init*/ false);
    let execution = ExecutionId::new("execution-1");
    assert_eq!(
        reconcile(
            &ledger,
            &execution,
            &Notify::new(),
            stopping.clone(),
            Duration::ZERO
        )
        .await,
        Err("model_access_revoked")
    );
    stop.send_replace(/*value*/ true);
    assert_eq!(
        reconcile(
            &ledger,
            &execution,
            &Notify::new(),
            stopping,
            END_RECONCILE_WAIT
        )
        .await,
        Ok(EndSessionReason::Cancelled)
    );
}

/// Returning from failed restore is not evidence that its original process stopped.
struct UnknownRestore;
impl crate::PriorRevisionRestore for UnknownRestore {
    /// A failed cleanup must transfer responsibility without a terminal result.
    async fn confirm_stop(&self, _execution: &ExecutionId) -> bool {
        false
    }
    /// No plugin is allowed to start after this restore failure.
    async fn restore(
        &self,
        _request: crate::RestoreRequest,
        _cancel: crate::RestoreCancel,
    ) -> Result<crate::Restored, crate::RestoreFailure> {
        Err(crate::RestoreFailure::Unavailable)
    }
}

/// The production driver transfers an unknown stop without terminal, queue settlement or history release.
#[test]
fn production_driver_defers_unknown_restore_without_releasing_terminal_history() {
    ora_logging::initialize_test_clock();
    let dispatcher = tracing::Dispatch::new(
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_test_writer()
            .finish(),
    );
    tracing::dispatcher::with_default(&dispatcher, || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                use crate::{AgentSessions, SessionConfig, SessionHost};
                use ora_node_protocol::*;
                let root = tempfile::tempdir().unwrap();
                let ledger = Ledger::default();
                let node = NodeRuntimeIdentity {
                    node_id: NodeId::new("node"),
                    incarnation_id: NodeIncarnationId::new("original"),
                };
                let execution = ExecutionId::new("8b0e5a52-6f1c-4c55-9d3e-2a7b1f0c9e41");
                let sessions = AgentSessions::new(
                    SessionConfig {
                        stop: Default::default(),
                        home_directory: root.path().into(),
                        deno_path: root.path().join("must-not-run"),
                        timezone: chrono_tz::UTC,
                        agent_ready_timeout: Duration::from_secs(/*secs*/ 1),
                        model_proxy: None,
                        workload: crate::SessionWorkload::Shared,
                    },
                    node.clone(),
                    ledger.clone(),
                    CheckoutPath(root.path().into()),
                    CheckoutPath(root.path().into()),
                    UnknownRestore,
                );
                sessions.start(
                    OperationId::new("operation"),
                    execution.clone(),
                    AgentSessionSpec {
                        host_mcp_servers: Vec::new(),
                        mcp_lifecycle: false,
                        mcp: Vec::new(),
                        workspace_directory: Vec::new(),
                        agent_configuration: None,
                        node_id: node.node_id.clone(),
                        agent_plugin_id: PluginId::new("official/ora-space.echo"),
                        agent_plugin_version: PluginVersion::new("1.0.0"),
                        checkout_execution_id: ExecutionId::new("clone"),
                        model_binding_id: None,
                        git_identity: GitIdentity {
                            name: "Fixture".into(),
                            email: "fixture@example.test".into(),
                        },
                        initial_turn: UserTurn {
                            turn_id: TurnId::new("initial"),
                            content: vec![ContentBlock::Text {
                                text: "must not be sent".into(),
                            }],
                        },
                        prior_revision: Some(PriorRevision {
                            revision_id: RevisionId::new("prior"),
                            final_commit: CommitId::new("0".repeat(40)),
                            git_repository_url: GitRepositoryUrl::parse(
                                "https://git.ora.test/tenant/project.git",
                            )
                            .expect("valid repository URL"),
                            delivery_ref: DeliveryRef::parse("refs/ora/deliveries/prior")
                                .expect("valid delivery ref"),
                            base_commit: CommitId::new("0".repeat(40)),
                        }),
                        trigger: None,
                    },
                );
                tokio::time::timeout(Duration::from_secs(/*secs*/ 2), ledger.deferred.notified())
                    .await
                    .unwrap();
                sessions.shutdown().await;
                let state = ledger.state.lock().unwrap();
                assert_eq!(state.ended, None);
                assert_eq!(
                    state.deferred,
                    Some(AgentSessionEnded {
                        node,
                        reason: AgentSessionEndReason::AgentFailed,
                        detail: Some("prior_revision_unavailable".into()),
                        plugin_id: None,
                        setting_id: None,
                        transport: None,
                        scheduled_result: None,
                        artifact_manifest: None,
                    })
                );
                assert!(state.settled.is_empty());
                assert!(!sessions.failed());
                drop(state);
                assert_eq!(
                    sessions.sealed_history(&execution),
                    Err(crate::session::HistoryUnavailable)
                );
            })
    });
}
