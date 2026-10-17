//! Which delivery runs next, decided on the worker that owns the ledger, and the executor thread
//! that runs its Git, file copies, pushes and uploads.
//!
//! One delivery step is outstanding at a time. Preparation and delivery are separate steps so the
//! worker persists the frozen plan between them: no push or PUT can start before the plan is
//! durable. Delivery pushes the frozen final commit, then uploads the history.
//! Shutdown, a lost Controller and a lifecycle cancellation never write a failure: an interrupted
//! step leaves the delivery running and the next start resumes it.
use super::agents::SessionHost;
use crate::git_server::GitServerAccess;
use crate::revision::{
    DELIVERY_ROOT, DeliveryGit, GrantStore, HttpUploader, ObjectUploader, Preparation, PushJob,
    Pushed, RetryPolicy, UploadEnd, UploadJob, prepare, push, upload,
};
use crate::{ManagedNode, SessionHost as _, Shutdown};
use gitlancer::GitRunner;
use ora_node_db::{DeliveryPlan, DeliveryProgress, FrozenOutcome, RevisionDelivery};
use ora_node_protocol::*;
use ora_utils::http::{ProxyConfig, ReqwestUploader, UploadOptions};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};

/// Generous enough for a large history on a slow link; a stuck request still ends and is retried.
const UPLOAD_OPTIONS: UploadOptions = UploadOptions {
    connect_timeout: Duration::from_secs(/*secs*/ 30),
    total_timeout: Duration::from_secs(/*secs*/ 30 * 60),
};

/// One step handed to the executor.
enum Step {
    Prepare(Box<Preparation>),
    Deliver(Box<(PushJob, UploadJob)>),
}

/// What a step reported; `Interrupted` leaves the delivery exactly as durable state describes it.
enum Finished {
    Prepared(ExecutionId, Result<Box<DeliveryPlan>, RevisionFailureCode>),
    /// The push failed definitively; nothing was uploaded.
    PushFailed(Box<UploadJob>, RevisionFailureCode),
    /// The push succeeded and the upload ended; `workspace_head` is the Workspace branch value
    /// the fast-forward established, for the ledger.
    Uploaded(Box<UploadJob>, UploadEnd, Option<Box<(PushJob, CommitId)>>),
    Interrupted(ExecutionId),
}

/// The worker's view of delivery: the executor, its one outstanding step and the frozen root.
pub(super) struct Revisions {
    steps: Option<mpsc::Sender<Step>>,
    results: mpsc::Receiver<Finished>,
    stop: tokio::sync::watch::Sender<bool>,
    thread: Option<JoinHandle<()>>,
    in_flight: Option<ExecutionId>,
    root: PathBuf,
}

/// Whether a delivery can be prepared now.
enum Inputs {
    Ready(Box<Preparation>),
    Failed(RevisionFailureCode),
    /// The session's history is still being released by its actor; try again shortly.
    Wait,
}

impl Revisions {
    /// Starts the executor when this Node can run delivery Git, i.e. when it has the clone
    /// configuration that created the checkouts. Frozen directories no running delivery owns are
    /// removed first: they belong to deliveries whose terminal evidence is already durable.
    pub(super) fn start(
        node: &ManagedNode,
        grants: GrantStore,
        shutdown: Shutdown,
    ) -> Result<Option<Self>, crate::Error> {
        let (Some(policy), Some(access)) = (node.delivery_git_policy()?, node.git_server()) else {
            return Ok(None);
        };
        let root = node.home_directory().join(DELIVERY_ROOT);
        sweep(&root, &node.database.recoverable_deliveries()?);
        let git = DeliveryGit::new(
            node.git
                .runner()
                .detached_clone_host()
                .map_err(ora_node_db::Error::from)?,
            policy,
        );
        let node_git = crate::managed::AsNode(
            node.git
                .runner()
                .detached_clone_host()
                .map_err(ora_node_db::Error::from)?,
        );
        let uploader =
            HttpUploader::new(ReqwestUploader::new(ProxyConfig::default()), UPLOAD_OPTIONS);
        Self::spawn(
            (git, node_git, access),
            uploader,
            grants,
            shutdown,
            root,
            RetryPolicy::DEFAULT,
        )
        .map(Some)
        .map_err(|error| ora_node_db::Error::from(error).into())
    }

    /// Runs steps on a dedicated thread with its own runtime for uploads; preparation runs
    /// outside that runtime because host-backed Git blocks on a runtime of its own.
    fn spawn<R, N, U>(
        (git, node_git, access): (DeliveryGit<R>, N, GitServerAccess),
        uploader: U,
        grants: GrantStore,
        shutdown: Shutdown,
        root: PathBuf,
        policy: RetryPolicy,
    ) -> std::io::Result<Self>
    where
        R: GitRunner + Send + 'static,
        N: GitRunner + Send + 'static,
        U: ObjectUploader,
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (steps, requests) = mpsc::channel::<Step>();
        let (reply, results) = mpsc::channel();
        let (stop, stopping) = tokio::sync::watch::channel(/*init*/ false);
        let frozen_root = root.clone();
        let thread = thread::Builder::new()
            .name("ora-node-revisions".into())
            .spawn(move || {
                for step in requests {
                    let finished = match step {
                        Step::Prepare(job) => {
                            let result = prepare(&git, &frozen_root, &job);
                            // Git refused by a stopping Node is not the delivery's failure.
                            if result.is_err() && shutdown.requested() {
                                Finished::Interrupted(job.execution)
                            } else {
                                Finished::Prepared(job.execution, result.map(Box::new))
                            }
                        }
                        Step::Deliver(jobs) => {
                            let (push_job, job) = *jobs;
                            // Pushing runs host-backed Git, which blocks on a runtime of its own,
                            // so it stays outside the upload runtime too.
                            let pushed = push(&node_git, &access, &push_job, &|| {
                                shutdown.requested() || *stopping.borrow()
                            });
                            match pushed {
                                Pushed::Interrupted => Finished::Interrupted(job.execution),
                                Pushed::Failed(failure) => {
                                    Finished::PushFailed(Box::new(job), failure)
                                }
                                Pushed::Delivered { workspace_head } => {
                                    let mut stopping = stopping.clone();
                                    let end = runtime.block_on(async {
                                        tokio::select! {
                                            biased;
                                            _ = stopping.wait_for(|stop| *stop) => None,
                                            end = upload(&uploader, &grants, &job, policy) => Some(end),
                                        }
                                    });
                                    match end {
                                        Some(end) => Finished::Uploaded(
                                            Box::new(job),
                                            end,
                                            workspace_head.map(|head| Box::new((push_job, head))),
                                        ),
                                        None => Finished::Interrupted(job.execution),
                                    }
                                }
                            }
                        }
                    };
                    if reply.send(finished).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            steps: Some(steps),
            results,
            stop,
            thread: Some(thread),
            in_flight: None,
            root,
        })
    }

    /// Persists finished steps, then hands the oldest runnable delivery to an idle executor.
    pub(super) fn advance(
        &mut self,
        node: &mut ManagedNode,
        agents: Option<&SessionHost>,
    ) -> Result<(), crate::Error> {
        while let Ok(finished) = self.results.try_recv() {
            self.in_flight = None;
            self.settle(node, finished)?;
        }
        if self.thread.as_ref().is_some_and(JoinHandle::is_finished) {
            return Err(crate::Error::Configuration(
                "Revision delivery executor stopped unexpectedly".into(),
            ));
        }
        if self.in_flight.is_some() {
            return Ok(());
        }
        for record in node.database.recoverable_deliveries()? {
            let step = match record.progress.clone() {
                DeliveryProgress::Frozen(plan) => match push_job(node, &record, &plan)? {
                    Some(push) => Step::Deliver(Box::new((
                        push,
                        UploadJob {
                            operation: record.command.operation_id.clone(),
                            execution: record.command.execution_id.clone(),
                            node_id: node.node_id().clone(),
                            directory: self.root.join(&plan.directory),
                            outcome: plan.outcome,
                        },
                    ))),
                    None => {
                        fail(
                            node,
                            &record.command.execution_id,
                            RevisionFailureCode::CheckoutUnavailable,
                        )?;
                        continue;
                    }
                },
                DeliveryProgress::Preparing => match inputs(node, agents, &record)? {
                    Inputs::Ready(job) => Step::Prepare(job),
                    Inputs::Failed(failure) => {
                        fail(node, &record.command.execution_id, failure)?;
                        continue;
                    }
                    Inputs::Wait => {
                        ora_logging::ora_debug!(execution_id = %record.command.execution_id.as_str(), "Revision delivery waits for its session to settle");
                        continue;
                    }
                },
                DeliveryProgress::Completed(_) => continue,
            };
            ora_logging::ora_info!(execution_id = %record.command.execution_id.as_str(), step = match &step { Step::Prepare(_) => "prepare", Step::Deliver(_) => "deliver" }, "Revision delivery step started");
            self.in_flight = Some(record.command.execution_id.clone());
            self.steps
                .as_ref()
                .and_then(|steps| steps.send(step).ok())
                .ok_or_else(|| {
                    crate::Error::Configuration("Revision delivery executor is unavailable".into())
                })?;
            break;
        }
        Ok(())
    }

    /// Stops uploads and waits for the executor, keeping whatever finished before the stop.
    /// Nothing unfinished is failed: the deliveries resume after restart.
    pub(super) fn finish(mut self, node: &mut ManagedNode) -> Result<(), crate::Error> {
        self.stop.send_replace(/*value*/ true);
        self.steps.take();
        if let Some(thread) = self.thread.take() {
            thread.join().map_err(|_| {
                crate::Error::Shutdown("Revision delivery executor panicked".into())
            })?;
        }
        while let Ok(finished) = self.results.try_recv() {
            self.settle(node, finished)?;
        }
        Ok(())
    }

    /// Makes one step's outcome durable before anything else may follow from it.
    fn settle(&self, node: &mut ManagedNode, finished: Finished) -> Result<(), crate::Error> {
        match finished {
            Finished::Prepared(execution, Ok(plan)) => {
                node.database.freeze_delivery(&execution, &plan)?;
            }
            Finished::Prepared(execution, Err(failure)) => fail(node, &execution, failure)?,
            Finished::PushFailed(job, failure) => {
                fail(node, &job.execution, failure)?;
                if let Err(error) = std::fs::remove_dir_all(&job.directory) {
                    ora_logging::ora_warn!(execution_id = %job.execution.as_str(), error = %error, "frozen delivery objects were not removed");
                }
            }
            Finished::Uploaded(job, end, workspace_head) => {
                // The ledger learns the branch value before the terminal result, so a later push
                // from a resumed checkout of the same Workspace declares it.
                if let Some((push, head)) = workspace_head.map(|boxed| *boxed)
                    && let Err(error) = node.database.record_branch_head(
                        &push.spec.checkout_execution_id,
                        &push.spec.workspace_ref,
                        &head,
                    )
                {
                    ora_logging::ora_warn!(execution_id = %job.execution.as_str(), error = %error, "Workspace branch value was not recorded");
                }
                let result = match end {
                    UploadEnd::Uploaded => job.outcome.result(),
                    UploadEnd::Failed => failed(node, RevisionFailureCode::UploadFailed),
                };
                node.database.complete_delivery(&job.execution, result)?;
                // The terminal event carries the evidence from here on; the bytes are no longer
                // needed, and a leftover directory is swept on the next start.
                if let Err(error) = std::fs::remove_dir_all(&job.directory) {
                    ora_logging::ora_warn!(execution_id = %job.execution.as_str(), error = %error, "frozen delivery objects were not removed");
                }
            }
            Finished::Interrupted(execution) => {
                ora_logging::ora_info!(execution_id = %execution.as_str(), "Revision delivery step interrupted; it resumes after restart");
            }
        }
        Ok(())
    }
}

impl Drop for Revisions {
    /// Error paths must not leave an uploader running after the database lease is released.
    fn drop(&mut self) {
        self.stop.send_replace(/*value*/ true);
        self.steps.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Resolves the session, checkout and history a delivery names, all from this Node's ledger.
///
/// The session must have its terminal result and no live actor (ADR D1). The Node cannot yet
/// prove that an agent plugin's descendants are gone: plugins run outside host Scopes, and the
/// session only ends after the plugin's own process tree was stopped (see `session::driver`).
fn inputs(
    node: &ManagedNode,
    agents: Option<&SessionHost>,
    record: &RevisionDelivery,
) -> Result<Inputs, crate::Error> {
    let spec = &record.command.payload.spec;
    let Some(session) = node.database.delivery_session(&spec.session_execution_id)? else {
        return Ok(Inputs::Failed(RevisionFailureCode::SessionNotSettled));
    };
    if !matches!(session.state, ExecutionState::Completed(_)) {
        return Ok(Inputs::Failed(RevisionFailureCode::SessionNotSettled));
    }
    let session_spec = session.command.payload.spec;
    // The checkout must be the session's own, at the base the delivery compares against, and
    // the Workspace branch the delivery fast-forwards must be the one it was cloned for.
    let Some(checkout) = checkout_of(node, spec)? else {
        return Ok(Inputs::Failed(RevisionFailureCode::CheckoutUnavailable));
    };
    if session_spec.checkout_execution_id != spec.checkout_execution_id {
        return Ok(Inputs::Failed(RevisionFailureCode::CheckoutUnavailable));
    }
    let Ok(history) = ora_history::history_path(
        &node.home_directory().join("sessions"),
        spec.session_execution_id.as_str(),
    ) else {
        return Ok(Inputs::Failed(RevisionFailureCode::HistoryUnavailable));
    };
    if !history.is_file() {
        return Ok(Inputs::Failed(RevisionFailureCode::HistoryUnavailable));
    }
    // The ledger records the terminal result just before the actor lets go of the history; an
    // existing file the host still refuses belongs to a session that is settling right now.
    if let Some(agents) = agents
        && agents.sealed_history(&spec.session_execution_id).is_err()
    {
        return Ok(Inputs::Wait);
    }
    Ok(Inputs::Ready(Box::new(Preparation {
        execution: record.command.execution_id.clone(),
        spec: spec.clone(),
        checkout,
        history,
        author: session_spec.git_identity,
        node: node.identity().clone(),
        owner: node.git.runner().process_config().workload_uid,
    })))
}

/// The delivery's checkout, when it is a successful clone at the delivery's base commit and for
/// its Workspace branch.
fn checkout_of(
    node: &ManagedNode,
    spec: &DeliverRevisionSpec,
) -> Result<Option<PathBuf>, crate::Error> {
    Ok(node
        .database
        .cloned_checkout(&spec.checkout_execution_id)?
        .filter(|checkout| {
            checkout.spec.base_commit == spec.base_commit
                && checkout.spec.workspace_ref == spec.workspace_ref
        })
        .map(|checkout| checkout.path))
}

/// What a frozen delivery pushes: the frozen final commit from its checkout, declaring the
/// ledger's last known Workspace branch value. `None` when the checkout is gone.
fn push_job(
    node: &ManagedNode,
    record: &RevisionDelivery,
    plan: &DeliveryPlan,
) -> Result<Option<PushJob>, crate::Error> {
    let spec = &record.command.payload.spec;
    let (Some(checkout), Some(config)) = (checkout_of(node, spec)?, node.clone_config()) else {
        return Ok(None);
    };
    let final_commit = match &plan.outcome {
        FrozenOutcome::Delivered(result) => result.final_commit.clone(),
        FrozenOutcome::Unchanged(result) => result.final_commit.clone(),
    };
    Ok(Some(PushJob {
        execution: record.command.execution_id.clone(),
        spec: spec.clone(),
        checkout,
        final_commit,
        workspace_head: node
            .database
            .branch_head(&spec.checkout_execution_id, &spec.workspace_ref)?,
        repository_root: config.repository_root.clone(),
    }))
}

/// Writes a definitive failure for the current incarnation.
fn fail(
    node: &mut ManagedNode,
    execution: &ExecutionId,
    failure: RevisionFailureCode,
) -> Result<(), crate::Error> {
    ora_logging::ora_warn!(execution_id = %execution.as_str(), failure = ?failure, "Revision delivery failed");
    let result = failed(node, failure);
    Ok(node.database.complete_delivery(execution, result)?)
}

/// A failure result reported by this incarnation.
fn failed(node: &ManagedNode, failure: RevisionFailureCode) -> RevisionExecutionResult {
    RevisionExecutionResult::RevisionFailed(RevisionFailed {
        node: node.identity().clone(),
        failure,
    })
}

/// Removes frozen directories that no frozen plan references. A preparing delivery never needs an
/// old directory: preparation recreates its own from scratch.
fn sweep(root: &Path, running: &[RevisionDelivery]) {
    let keep: HashSet<String> = running
        .iter()
        .filter_map(|record| match &record.progress {
            DeliveryProgress::Frozen(plan) => Some(plan.directory.clone()),
            DeliveryProgress::Preparing | DeliveryProgress::Completed(_) => None,
        })
        .collect();
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !keep.contains(&name) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}
