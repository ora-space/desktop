use super::{PromptEndCause, PromptEndReport, RuntimeCommand};
use crate::RuntimeError;
use std::{future::Future, pin::Pin};
use tokio::sync::{mpsc, oneshot};

type Cleanup =
    Box<dyn FnOnce() -> Pin<Box<dyn Future<Output = Result<(), RuntimeError>> + Send>> + Send>;

/// Owns one operation; explicit cancellation waits for the owner and workflow cleanup.
pub struct SessionEventStream<Event> {
    state: StreamState<Event>,
    /// How the runtime says the owning prompt ended; absent on non-prompt streams.
    end_report: Option<PromptEndReport>,
}

/// Cleanup ownership moves once from the receiver to a worker, then to a retained receipt.
enum StreamState<Event> {
    Active {
        receiver: mpsc::Receiver<Result<Event, RuntimeError>>,
        commands: Option<(mpsc::UnboundedSender<RuntimeCommand>, u64)>,
        cleanup: Option<Cleanup>,
    },
    Stopping(tokio::task::JoinHandle<Result<(), RuntimeError>>),
    Completed(Result<(), RuntimeError>),
}

impl<Event: Send + 'static> SessionEventStream<Event> {
    /// Builds a stream tied to one actor operation generation.
    pub(super) fn new(
        receiver: mpsc::Receiver<Result<Event, RuntimeError>>,
        commands: mpsc::UnboundedSender<RuntimeCommand>,
        operation_id: u64,
    ) -> Self {
        Self {
            state: StreamState::Active {
                receiver,
                commands: Some((commands, operation_id)),
                cleanup: None,
            },
            end_report: None,
        }
    }

    /// Attaches the report of how this prompt's turn ended, written by the actor.
    pub(super) fn with_end_report(mut self, report: PromptEndReport) -> Self {
        self.end_report = Some(report);
        self
    }

    /// Returns how the runtime says this prompt ended, once its turn settled.
    ///
    /// A consumer that ends the operation itself, such as a workflow park reading the cause while
    /// its cleanup runs, keeps this handle and reads it after the stream stopped.
    pub fn end_cause(&self) -> Option<PromptEndCause> {
        self.end_report.as_ref().and_then(PromptEndReport::recorded)
    }

    /// Hands the turn owner the same report the actor writes, so cleanup can read it later.
    pub fn end_report(&self) -> Option<PromptEndReport> {
        self.end_report.clone()
    }

    /// Runs workflow restoration after the operation owner has settled, retaining the first failure.
    pub fn attach_cleanup<F, Fut>(mut self, cleanup: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), RuntimeError>> + Send + 'static,
    {
        let StreamState::Active {
            cleanup: attached, ..
        } = &mut self.state
        else {
            return self;
        };
        let existing = attached.take();
        *attached = Some(Box::new(move || {
            Box::pin(async move {
                let previous = match existing {
                    Some(existing) => existing().await,
                    None => Ok(()),
                };
                let result = cleanup().await;
                previous.and(result)
            })
        }));
        self
    }

    /// Uses the publisher's cleanup future to confirm its owned worker has exited.
    pub fn with_cleanup<F, Fut>(
        receiver: mpsc::Receiver<Result<Event, RuntimeError>>,
        cleanup: F,
    ) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), RuntimeError>> + Send + 'static,
    {
        Self {
            state: StreamState::Active {
                receiver,
                commands: None,
                cleanup: Some(Box::new(move || Box::pin(cleanup()))),
            },
            end_report: None,
        }
    }

    /// Receives the next ordered runtime event.
    pub async fn recv(&mut self) -> Option<Result<Event, RuntimeError>> {
        match &mut self.state {
            StreamState::Active { receiver, .. } => receiver.recv().await,
            StreamState::Stopping(_) | StreamState::Completed(_) => None,
        }
    }

    /// Returns a buffered item without waiting during HTTP shutdown.
    pub fn try_recv(&mut self) -> Option<Result<Event, RuntimeError>> {
        match &mut self.state {
            StreamState::Active { receiver, .. } => receiver.try_recv().ok(),
            StreamState::Stopping(_) | StreamState::Completed(_) => None,
        }
    }

    /// Requests cancellation once and waits for owner cleanup followed by workflow restoration.
    /// Dropping this wait does not abort cleanup; a later call observes the same result.
    /// Callers may bound their wait with a timeout, which never means cleanup succeeded.
    pub async fn cancel_and_wait(&mut self) -> Result<(), RuntimeError> {
        if matches!(self.state, StreamState::Active { .. }) {
            // If starting the worker unwinds, retain an explicit failure instead of a false receipt.
            let previous = std::mem::replace(
                &mut self.state,
                StreamState::Completed(Err(super::support::runtime_unavailable())),
            );
            if let StreamState::Active {
                receiver,
                commands,
                cleanup,
            } = previous
            {
                self.state = StreamState::Stopping(start_cleanup(receiver, commands, cleanup));
            }
        }
        let result = match &mut self.state {
            StreamState::Stopping(worker) => match worker.await {
                Ok(result) => result,
                Err(error) => Err(RuntimeError::internal(
                    "stream cleanup task did not confirm completion",
                    error,
                )),
            },
            StreamState::Completed(result) => return result.clone(),
            StreamState::Active { .. } => return Err(super::support::runtime_unavailable()),
        };
        self.state = StreamState::Completed(result.clone());
        result
    }
}

/// Keeps cancellation and cleanup alive even when the consumer abandons its waiter.
fn start_cleanup<Event: Send + 'static>(
    mut receiver: mpsc::Receiver<Result<Event, RuntimeError>>,
    commands: Option<(mpsc::UnboundedSender<RuntimeCommand>, u64)>,
    cleanup: Option<Cleanup>,
) -> tokio::task::JoinHandle<Result<(), RuntimeError>> {
    tokio::spawn(async move {
        let owner = if let Some((commands, operation_id)) = commands {
            let (completion, confirmed) = oneshot::channel();
            if commands
                .send(RuntimeCommand::Cancel {
                    operation_id,
                    completion: Some(completion),
                })
                .is_err()
            {
                Err(super::support::runtime_unavailable())
            } else {
                // Keep draining while the actor settles; a full event queue must not deadlock it.
                let drain = async { while receiver.recv().await.is_some() {} };
                let (confirmed, ()) = tokio::join!(confirmed, drain);
                confirmed.unwrap_or_else(|error| {
                    Err(RuntimeError::internal(
                        "stream owner did not confirm cleanup",
                        error,
                    ))
                })
            }
        } else {
            Ok(())
        };
        drop(receiver);
        let restored = match cleanup {
            Some(cleanup) => cleanup().await,
            None => Ok(()),
        };
        match (owner, restored) {
            (Err(owner), Err(restoration)) => Err(RuntimeError::new(
                owner.classification(),
                owner.public_error().clone(),
                format!("{owner}; workflow restoration also failed: {restoration}"),
            )),
            (owner, restored) => owner.and(restored),
        }
    })
}

impl<Event> Drop for SessionEventStream<Event> {
    /// Drop is best effort only; consumers needing confirmation must use cancel_and_wait.
    fn drop(&mut self) {
        if let StreamState::Active {
            commands, cleanup, ..
        } = &mut self.state
        {
            if let Some((commands, operation_id)) = commands.take() {
                let _ = commands.send(RuntimeCommand::Cancel {
                    operation_id,
                    completion: None,
                });
            }
            if let Some(cleanup) = cleanup.take()
                && let Ok(runtime) = tokio::runtime::Handle::try_current()
            {
                runtime.spawn(cleanup());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionEventStream;
    use crate::RuntimeError;
    use tokio::sync::mpsc;

    /// Verifies a buffered terminal error is visible without waiting on `recv`.
    #[tokio::test]
    async fn try_recv_returns_a_buffered_error_without_waiting() {
        let (sender, receiver) = mpsc::channel::<Result<(), RuntimeError>>(1);
        let mut stream = SessionEventStream::with_cleanup(receiver, || async { Ok(()) });
        sender
            .try_send(Err(RuntimeError::internal(
                "stream interrupted",
                std::io::Error::other("closed"),
            )))
            .expect("buffered error is queued");

        assert!(matches!(stream.try_recv(), Some(Err(_))));
        assert!(stream.try_recv().is_none());
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    /// An actor receipt cannot bypass blocked workflow restoration, and retries share its result.
    #[tokio::test]
    async fn cancellation_waits_for_owner_and_restoration_once() {
        let (events, receiver) = mpsc::channel::<Result<(), RuntimeError>>(1);
        let (commands, mut requests) = mpsc::unbounded_channel();
        let (entered, entering) = oneshot::channel();
        let (release, blocked) = oneshot::channel();
        let mut stream = SessionEventStream::new(receiver, commands, /*operation_id*/ 17)
            .attach_cleanup(move || async move {
                entered.send(()).expect("restoration entered");
                blocked.await.expect("release restoration");
                Ok(())
            });
        let mut waiter = tokio::spawn(async move {
            let first = stream.cancel_and_wait().await;
            let second = stream.cancel_and_wait().await;
            (first.is_ok(), second.is_ok())
        });
        let RuntimeCommand::Cancel {
            operation_id,
            completion: Some(completion),
        } = requests.recv().await.expect("cancel requested")
        else {
            panic!("expected acknowledged cancellation")
        };
        assert_eq!(operation_id, 17);
        assert!(!waiter.is_finished());
        completion.send(Ok(())).expect("owner confirms");
        // Even the actor receipt cannot stand in for release of its event publisher.
        assert!(!waiter.is_finished());
        drop(events);
        entering.await.expect("restoration started");
        tokio::select! {
            biased;
            _ = &mut waiter => panic!("returned before restoration"),
            () = std::future::ready(()) => {}
        }
        release.send(()).expect("release cleanup");
        assert_eq!(waiter.await.expect("waiter joined"), (true, true));
        assert!(requests.recv().await.is_none());
    }

    /// Cleanup failures and missing actor receipts stay failures on every subsequent wait.
    #[tokio::test]
    async fn cancellation_preserves_failure_and_missing_confirmation() {
        for confirm in [true, false] {
            let (events, receiver) = mpsc::channel::<Result<(), RuntimeError>>(1);
            let (commands, mut requests) = mpsc::unbounded_channel();
            let mut stream = SessionEventStream::new(receiver, commands, /*operation_id*/ 19)
                .attach_cleanup(|| async {
                    Err(RuntimeError::internal(
                        "restoration failed",
                        std::io::Error::other("fixture"),
                    ))
                });
            let waiter = tokio::spawn(async move {
                let first = stream
                    .cancel_and_wait()
                    .await
                    .expect_err("must fail")
                    .to_string();
                let second = stream
                    .cancel_and_wait()
                    .await
                    .expect_err("retry must fail")
                    .to_string();
                assert_eq!(first, second);
                first
            });
            let RuntimeCommand::Cancel {
                completion: Some(completion),
                ..
            } = requests.recv().await.expect("cancel")
            else {
                panic!("expected cancellation")
            };
            if confirm {
                completion.send(Ok(())).expect("confirm owner");
            } else {
                drop(completion);
            }
            drop(events);
            let error = waiter.await.expect("join waiter");
            assert!(error.contains(if confirm {
                "restoration failed"
            } else {
                "did not confirm"
            }));
        }
    }

    /// Abandoning a wait keeps the owned task alive and allows a later wait to observe completion.
    #[tokio::test]
    async fn abandoned_wait_does_not_abort_cleanup() {
        let (events, receiver) = mpsc::channel::<Result<(), RuntimeError>>(1);
        drop(events);
        let (release, blocked) = oneshot::channel();
        let mut stream = SessionEventStream::with_cleanup(receiver, move || async move {
            blocked.await.expect("release cleanup");
            Ok(())
        });
        {
            let wait = stream.cancel_and_wait();
            tokio::pin!(wait);
            tokio::select! { biased; _ = &mut wait => panic!("premature confirmation"), () = std::future::ready(()) => {} }
        }
        release.send(()).expect("cleanup still alive");
        stream.cancel_and_wait().await.expect("confirmed cleanup");
    }
    /// A cleanup worker panic is an unconfirmed failure and stays failed on repeated waits.
    #[tokio::test]
    async fn cleanup_panic_cannot_report_success() {
        let (events, receiver) = mpsc::channel::<Result<(), RuntimeError>>(1);
        drop(events);
        let mut stream = SessionEventStream::with_cleanup(receiver, || async {
            panic!("fixture cleanup panic");
        });
        let first = stream
            .cancel_and_wait()
            .await
            .expect_err("panic must fail")
            .to_string();
        let second = stream
            .cancel_and_wait()
            .await
            .expect_err("repeat must fail")
            .to_string();
        assert_eq!(first, second);
        assert!(first.contains("did not confirm completion"));
    }
}
