//! The interfaces a session execution shares with the rest of the Node.
//!
//! Session execution, the Node ledger, plugin installation and clone bookkeeping are built
//! independently, so each side depends only on these traits: a session is tested against an
//! in-memory ledger, and the ledger against a fake [`SessionHost`].

use ora_node_protocol::{
    AgentSessionEnded, AgentSessionSpec, CommandId, CommitId, EndSessionReason, ExecutionId,
    OperationId, PluginId, PluginVersion, PriorRevision, Sequence, ThreadEvent, UserTurn,
};
use std::error::Error;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// One session command the protocol side accepted and persisted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionCommand {
    SubmitUserTurn(UserTurn),
    EndSession(EndSessionReason),
}

/// A persisted command that the session has not settled yet.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QueuedCommand {
    pub command_id: CommandId,
    pub command: SessionCommand,
}

/// How the session settled one command; each command is settled at most once.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandSettlement {
    Executed,
    /// An `EndSession` arrived before this user turn ran, so it never will.
    Discarded,
}

/// The Node ledger as a session execution sees it.
///
/// Every method is a short synchronous durable write or read. `append_thread_event` is called
/// from inside the session's actor right after the record reached the session history, so the
/// ledger decides only the sequence and durability; sending, the in-flight window and replay
/// belong to the protocol side.
pub trait SessionLedger: Clone + Send + Sync + 'static {
    type Error: Error + Send + Sync + 'static;

    /// Makes possible provider history durable before asking the provider to create a session.
    fn admit_history(&self, _execution: &ExecutionId) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Persists actual process-stop evidence; recovery must not call this to fabricate quiescence.
    fn confirm_writer_stop(&self, _execution: &ExecutionId) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Assigns the next sequence to one Thread event and returns once it is durable.
    fn append_thread_event(
        &self,
        execution: &ExecutionId,
        event: ThreadEvent,
    ) -> Result<Sequence, Self::Error>;

    /// Writes the terminal result and its event; every later append for the execution fails.
    ///
    /// A command persisted after the session settled its queue is still queued here, and the
    /// ledger settles it as discarded together with the terminal result.
    fn end_session(
        &self,
        execution: &ExecutionId,
        ended: AgentSessionEnded,
    ) -> Result<Sequence, Self::Error>;

    /// History may be released only after the durable terminal, including deferred restores.
    fn session_completed(&self, _execution: &ExecutionId) -> Result<bool, Self::Error> {
        Ok(true)
    }

    /// Persists a terminal intention while original restore stop evidence is still pending.
    fn defer_restore_end(
        &self,
        _execution: &ExecutionId,
        _ended: &AgentSessionEnded,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Persists restoration success before plugin admission.
    fn restored(&self, _execution: &ExecutionId) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Records admission before a plugin can have a side effect.
    fn agent_admitted(&self, _execution: &ExecutionId) -> Result<(), Self::Error> {
        Ok(())
    }

    /// Returns every queued command of the execution in acceptance order.
    ///
    /// The whole queue rather than its head, because an `EndSession` accepted behind queued user
    /// turns must be seen while a turn is still running.
    fn queued_commands(&self, execution: &ExecutionId) -> Result<Vec<QueuedCommand>, Self::Error>;

    /// Settles one queued command.
    fn settle_command(
        &self,
        execution: &ExecutionId,
        command_id: &CommandId,
        settlement: CommandSettlement,
    ) -> Result<(), Self::Error>;
}

/// Resolves the checkout a successful clone execution left on this Node.
///
/// Session and delivery both go through here, so neither ever composes a checkout path itself.
pub trait CheckoutResolver: Send + Sync + 'static {
    /// Returns the checkout and the snapshot input it was made from, or `None` when the
    /// execution did not leave one.
    fn checkout(&self, clone_execution: &ExecutionId) -> Option<ora_node_db::ClonedCheckout>;
}

/// The installed plugins a session may run, as the plugin installer maintains them.
pub trait PluginCatalog: Send + Sync + 'static {
    /// Keeps a plugin from being installed over or removed while it is held; dropping releases it.
    type Lease: Send + 'static;

    /// Takes a use lease on one plugin; install and removal report `plugin_in_use` while it lives.
    fn lease(&self, plugin_id: &PluginId) -> Self::Lease;

    /// Returns the package directory of exactly this installed version, if present.
    fn installed(&self, plugin_id: &PluginId, version: &PluginVersion) -> Option<PathBuf>;

    /// Proves the installed package is the artifact Cloud froze for a configured session, that it
    /// still carries the frozen declaration, that the Settings are valid for it, and that it
    /// offers the frozen protocol. Called under the session's lease, before model authority.
    fn check_configuration(
        &self,
        configuration: &ora_node_protocol::FrozenAgentConfiguration,
    ) -> Result<(), InstalledConfigurationError>;
}

/// Why the installed package cannot run a frozen configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstalledConfigurationError {
    /// The package, its declaration or the Settings are not the ones Cloud froze.
    Invalid,
    /// The installed declaration does not offer the protocol Cloud froze.
    ProtocolMismatch,
}

impl InstalledConfigurationError {
    /// The bounded session failure code for this refusal.
    pub fn code(self) -> &'static str {
        match self {
            Self::Invalid => "agent_model_configuration_invalid",
            Self::ProtocolMismatch => "agent_model_protocol_mismatch",
        }
    }
}

/// The session history of an execution cannot be delivered yet, or at all.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("session history is unavailable")]
pub struct HistoryUnavailable;

/// One prior Revision to restore into a session's checkout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoreRequest {
    pub operation: OperationId,
    /// The session execution, whose ID the Git Server authorizes the fetch by.
    pub execution: ExecutionId,
    /// The checkout the session resolved; nothing has been handed to the workload user yet.
    pub checkout: PathBuf,
    /// The checkout's Workspace branch short name, which moves to the prior final commit.
    pub branch: String,
    /// The clone's base commit: the snapshot the prior Revision's base is judged against.
    pub base_commit: CommitId,
    pub prior: PriorRevision,
}

/// Tells a running restore that its session is ending: the Node stops or an `EndSession`
/// arrived before the agent started.
#[derive(Clone, Debug, Default)]
pub struct RestoreCancel(Arc<AtomicBool>);

impl RestoreCancel {
    /// Asks the restore to stop at its next step.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether the restore was asked to stop.
    pub fn requested(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// What a successful restore left: the checkout's branch at the prior final commit, clean.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Restored {
    /// The prior Revision's base commit is in the history of this Workspace's snapshot.
    OnSnapshotHistory,
    /// The external repository's history was rewritten: this Workspace's snapshot does not
    /// contain `base_commit`, the base the restored `final_commit` was built on.
    Diverged {
        final_commit: CommitId,
        base_commit: CommitId,
        branch: String,
    },
}

/// Why a prior Revision could not be restored; the session ends `agent_failed` with this detail.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RestoreFailure {
    /// The prior final commit could not be fetched, verified or checked out. Possibly transient:
    /// Cloud offers the same Revision to the next run.
    #[error("prior Revision is unavailable")]
    Unavailable,
    /// The Git Server has no readable delivery ref for the prior Revision, so its Git content is
    /// permanently out of reach and Cloud stops offering it.
    #[error("the prior Revision's base commit is unavailable")]
    BaseUnavailable,
}

impl RestoreFailure {
    /// The bounded code the session end reports (restore contract D3).
    pub fn detail(self) -> &'static str {
        match self {
            Self::Unavailable => "prior_revision_unavailable",
            Self::BaseUnavailable => "prior_revision_base_unavailable",
        }
    }
}

/// Restores the prior Revision a session resumes into its checkout before the agent starts.
///
/// The session driver calls it at most once per session, after resolving the checkout and before
/// handing the checkout to the workload user or starting any plugin, and only when the session
/// input names a prior Revision. Implementations fetch and verify the prior commit, run Git with the
/// deployment's hardened policy off the async workers, and leave nothing behind on failure but
/// what Git itself wrote into the checkout; the driver ends the session on any failure.
///
/// Once `cancel` is set, implementations start no further Git and return as soon as what they
/// already started has ended: the driver awaits that before the session may end, so a delivery
/// never reads a checkout a restore is still writing.
/// what Git itself wrote into the checkout. The driver waits for original stop proof before ending;
/// an expired proof wait transfers durable responsibility without a terminal result.
pub trait PriorRevisionRestore: Send + Sync + 'static {
    /// Anchors the stop clock before cancellation can reach the command boundary.
    fn request_stop(&self, _execution: &ExecutionId) {}

    /// Waits for original stop proof, with its independent durable bound.
    fn confirm_stop(&self, _execution: &ExecutionId) -> impl Future<Output = bool> + Send {
        async { true }
    }

    /// Leaves the checkout's branch at the prior final commit, or says why it could not.
    fn restore(
        &self,
        request: RestoreRequest,
        cancel: RestoreCancel,
    ) -> impl Future<Output = Result<Restored, RestoreFailure>> + Send;
}

/// A Node without restore never advertises it, so the Controller sends it no resumed session;
/// one that arrives anyway cannot be restored and ends as unavailable.
impl<R: PriorRevisionRestore> PriorRevisionRestore for Option<R> {
    fn request_stop(&self, execution: &ExecutionId) {
        if let Some(restorer) = self {
            restorer.request_stop(execution);
        }
    }
    async fn confirm_stop(&self, execution: &ExecutionId) -> bool {
        match self {
            Some(restorer) => restorer.confirm_stop(execution).await,
            None => true,
        }
    }
    /// Delegates to the configured restorer, if any.
    async fn restore(
        &self,
        request: RestoreRequest,
        cancel: RestoreCancel,
    ) -> Result<Restored, RestoreFailure> {
        match self {
            Some(restorer) => restorer.restore(request, cancel).await,
            None => Err(RestoreFailure::Unavailable),
        }
    }
}

/// Session execution as the protocol and delivery sides call it.
pub trait SessionHost {
    /// Starts a session whose input is already persisted and returns without waiting for it.
    fn start(&self, operation: OperationId, execution: ExecutionId, spec: AgentSessionSpec);

    /// Wakes the session to read its queued commands; a lost wake never loses a command.
    fn command_arrived(&self, execution: &ExecutionId);

    /// Ends a session the Node found without a terminal result after restarting.
    ///
    /// Called before any new session command is accepted. The returned result is written by the
    /// ledger.
    fn recover_interrupted(&self, execution: &ExecutionId) -> AgentSessionEnded;

    /// Returns the session history once the session has ended and nothing writes to it anymore.
    fn sealed_history(&self, execution: &ExecutionId) -> Result<PathBuf, HistoryUnavailable>;
}
