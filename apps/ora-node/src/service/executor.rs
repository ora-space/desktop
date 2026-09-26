//! The clone executor: the one thread that waits on the host, so the worker never does.
//!
//! It performs one step at a time and owns no Node database connection; every step it receives
//! was made durable by the worker first. The worker owns this thread and drains it before the
//! Node's final cleanup.
use crate::managed::CloneHost;
use crate::repository::{CloneStep, CloneStepResult};
use std::{
    sync::mpsc,
    thread::{self, JoinHandle},
    time::Duration,
};

/// A running executor thread with its request and result channels.
pub(super) struct Executor {
    steps: Option<mpsc::Sender<CloneStep>>,
    results: mpsc::Receiver<CloneStepResult>,
    thread: Option<JoinHandle<()>>,
}

impl Executor {
    /// Starts the thread; it ends when the worker drops its request channel.
    pub(super) fn spawn(host: CloneHost) -> std::io::Result<Self> {
        let (steps, requests) = mpsc::channel::<CloneStep>();
        let (reply, results) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("ora-node-clone".into())
            .spawn(move || {
                for step in requests {
                    if reply.send(step.perform(&host)).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            steps: Some(steps),
            results,
            thread: Some(thread),
        })
    }

    /// Hands one durable step to the thread without waiting for it.
    pub(super) fn submit(&self, step: CloneStep) -> Result<(), String> {
        self.steps
            .as_ref()
            .and_then(|steps| steps.send(step).ok())
            .ok_or_else(|| "clone executor stopped unexpectedly".to_owned())
    }

    /// Returns a finished step if one is ready.
    ///
    /// A vanished thread is an unexpected failure, such as a panic, and must stop the Node visibly
    /// rather than leave an execution silently in flight.
    pub(super) fn try_result(&self) -> Result<Option<CloneStepResult>, String> {
        match self.results.try_recv() {
            Ok(result) => Ok(Some(result)),
            Err(mpsc::TryRecvError::Empty) => Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("clone executor stopped unexpectedly".into())
            }
        }
    }

    /// Waits up to `limit` for a finished step; `None` means it did not finish in time.
    pub(super) fn wait_result(&self, limit: Duration) -> Result<Option<CloneStepResult>, String> {
        match self.results.recv_timeout(limit) {
            Ok(result) => Ok(Some(result)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("clone executor stopped unexpectedly".into())
            }
        }
    }

    /// Closes the request channel and joins the thread when it has nothing left in flight.
    ///
    /// A step still waiting on an unresponsive host is not joined: process exit ends that thread,
    /// and the next start settles its Run from the recorded attempt.
    pub(super) fn finish(mut self, idle: bool) {
        self.steps.take();
        if let Some(thread) = self.thread.take()
            && idle
            && thread.join().is_err()
        {
            ora_logging::ora_warn!("clone executor panicked while stopping");
        }
    }
}
