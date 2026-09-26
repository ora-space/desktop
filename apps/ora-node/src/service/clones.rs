//! Which clone the executor works on next, decided on the worker that owns the Node database.
//!
//! Recovery passes and new admissions only queue executions; the worker starts the next one when
//! the executor is idle. At most one step is outstanding at a time, and the execution it belongs
//! to is never driven again until that step's result has been persisted.
use super::{executor::Executor, *};
use crate::{ManagedNode, NodeState, repository::CloneStep};
use std::{collections::VecDeque, time::Instant};

/// The worker's view of clone execution: the executor, its one outstanding step and the queue.
pub(super) struct Clones {
    executor: Executor,
    in_flight: Option<ExecutionId>,
    queue: VecDeque<(OperationId, ExecutionId)>,
    previous: Option<NodeState>,
}

impl Clones {
    /// Starts the executor with its own host side; the worker keeps the database.
    pub(super) fn start(node: &ManagedNode) -> std::io::Result<Self> {
        Ok(Self {
            executor: Executor::spawn(node.git.runner().detached_clone_host()?)?,
            in_flight: None,
            queue: VecDeque::new(),
            previous: None,
        })
    }

    /// Queues a newly admitted execution so it starts without waiting for the next recovery pass.
    pub(super) fn admit(&mut self, operation: OperationId, execution: ExecutionId) {
        if !self.queue.iter().any(|(_, queued)| *queued == execution) {
            self.queue.push_back((operation, execution));
        }
    }

    /// Begins a recovery pass over every recoverable clone once the previous pass has finished.
    ///
    /// Taking the snapshot only while nothing is queued or in flight lets each execution take one
    /// turn per pass, so an execution that always needs the host cannot starve the others.
    pub(super) fn recovery_pass(&mut self, node: &mut ManagedNode) -> Result<(), String> {
        if self.in_flight.is_some() || !self.queue.is_empty() {
            return Ok(());
        }
        for record in node
            .database
            .recoverable_clones()
            .map_err(|e| e.to_string())?
        {
            self.admit(record.command.operation_id, record.command.execution_id);
        }
        if self.queue.is_empty() {
            self.report(node)?;
        }
        Ok(())
    }

    /// Persists finished steps and keeps the executor busy with the next durable step.
    pub(super) fn advance(&mut self, node: &mut ManagedNode) -> Result<(), String> {
        while let Some(result) = self.executor.try_result()? {
            self.resume(node, result)?;
        }
        while self.in_flight.is_none() {
            let Some((operation, execution)) = self.queue.pop_front() else {
                break;
            };
            // Re-read the record: the queue holds identities, and only the database holds the
            // phase a step may start from.
            let Some(record) = node
                .database
                .find_clone(&operation, &execution)
                .map_err(|e| e.to_string())?
            else {
                continue;
            };
            let step = node.begin_clone(record).map_err(|e| e.to_string())?;
            self.submit(step)?;
            if self.in_flight.is_none() && self.queue.is_empty() {
                self.report(node)?;
            }
        }
        Ok(())
    }

    /// Waits a bounded time for the outstanding step during shutdown so its observation is kept.
    ///
    /// Follow-up steps of the same execution may still run (inspection after a successful exit);
    /// no new execution starts. What does not finish in time is settled on the next start.
    pub(super) fn drain(mut self, node: &mut ManagedNode, limit: Duration) -> Result<(), String> {
        let deadline = Instant::now() + limit;
        let result = (|| {
            while self.in_flight.is_some() {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let Some(result) = self.executor.wait_result(remaining)? else {
                    ora_logging::ora_warn!(
                        "clone step did not finish before shutdown; recovery settles it on restart"
                    );
                    break;
                };
                self.resume(node, result)?;
            }
            Ok(())
        })();
        let idle = self.in_flight.is_none();
        self.executor.finish(idle);
        result
    }

    /// Persists one result and submits the step that follows it, if any.
    fn resume(
        &mut self,
        node: &mut ManagedNode,
        result: crate::repository::CloneStepResult,
    ) -> Result<(), String> {
        if self.in_flight.as_ref() != Some(result.execution()) {
            // Only one step is ever outstanding, so this cannot happen; dropping it keeps a stray
            // observation from rewriting progress it was not taken from.
            ora_logging::ora_warn!(execution = %result.execution().as_str(), "discarded a clone result that was not in flight");
            return Ok(());
        }
        self.in_flight = None;
        let next = node.resume_clone(result).map_err(|e| e.to_string())?;
        self.submit(next)?;
        if self.in_flight.is_none() && self.queue.is_empty() {
            self.report(node)?;
        }
        Ok(())
    }

    /// Marks the step's execution in flight before handing it to the executor.
    fn submit(&mut self, step: Option<CloneStep>) -> Result<(), String> {
        if let Some(step) = step {
            self.in_flight = Some(step.execution().clone());
            self.executor.submit(step)?;
        }
        Ok(())
    }

    /// Refreshes admission visibility at the end of a pass and logs when it changes.
    fn report(&mut self, node: &mut ManagedNode) -> Result<(), String> {
        node.refresh_clone_state().map_err(|e| e.to_string())?;
        let state = node.state();
        if self.previous != Some(state) {
            ora_logging::ora_info!(state = ?state, "Node recovery pass completed");
            self.previous = Some(state);
        }
        Ok(())
    }
}
