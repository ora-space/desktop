#![allow(clippy::unwrap_used)]

use super::*;
use crate::model_proxy::{ModelAccess, ModelAccessStatus, tests::Gateway};
use crate::session::ports::{QueuedCommand, SessionCommand};
use ora_node_protocol::{AgentSessionEnded, CommandId, ModelBindingId, Sequence, ThreadEvent};
use pretty_assertions::assert_eq;
use std::sync::{Arc, Mutex, atomic::Ordering};

/// The ending path must return before plugin discovery or process launch.
struct CheckoutPath(std::path::PathBuf);
impl crate::CheckoutResolver for CheckoutPath {
    /// Resolves the isolated checkout without invoking Git for an unstarted session.
    fn checkout(&self, _execution: &ExecutionId) -> Option<std::path::PathBuf> {
        Some(self.0.clone())
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
}

#[derive(Default)]
struct State {
    queued: Vec<QueuedCommand>,
    settled: Vec<(CommandId, CommandSettlement)>,
    ended: Option<AgentSessionEnded>,
}

#[derive(Clone, Default)]
struct Ledger {
    state: Arc<Mutex<State>>,
    read: Arc<Notify>,
    terminal: Arc<Notify>,
}

impl SessionLedger for Ledger {
    type Error = std::io::Error;

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
    );
    sessions.start(
        execution.clone(),
        AgentSessionSpec {
            node_id: node.node_id.clone(),
            agent_plugin_id: PluginId::new("official/ora-space.echo"),
            agent_plugin_version: PluginVersion::new("1.0.0"),
            checkout_execution_id: ExecutionId::new("clone-1"),
            model_binding_id: Some(ModelBindingId::new("binding-1")),
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
            detail: None
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
