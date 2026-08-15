use crate::BackendError;
use ora_logging::ora_debug;
use tokio::sync::mpsc;

use super::RuntimeCommand;

/// Owns one finite business-event stream and cancels its operation when consumption stops early.
pub struct SessionEventStream<Event> {
    receiver: mpsc::Receiver<Result<Event, BackendError>>,
    commands: Option<mpsc::UnboundedSender<RuntimeCommand>>,
    operation_id: Option<u64>,
    cleanup: Option<Box<dyn FnOnce() + Send + 'static>>,
    completed: bool,
}

impl<Event> SessionEventStream<Event> {
    /// Builds a stream tied to one actor operation generation.
    pub(super) fn new(
        receiver: mpsc::Receiver<Result<Event, BackendError>>,
        commands: mpsc::UnboundedSender<RuntimeCommand>,
        operation_id: u64,
    ) -> Self {
        Self {
            receiver,
            commands: Some(commands),
            operation_id: Some(operation_id),
            cleanup: None,
            completed: false,
        }
    }

    /// Builds a stream owned by a non-actor publisher, such as the application event hub.
    pub(crate) fn with_cleanup<F>(
        receiver: mpsc::Receiver<Result<Event, BackendError>>,
        cleanup: F,
    ) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        Self {
            receiver,
            commands: None,
            operation_id: None,
            cleanup: Some(Box::new(cleanup)),
            completed: false,
        }
    }

    /// Receives the next ordered event or terminal error from the backend actor.
    pub async fn recv(&mut self) -> Option<Result<Event, BackendError>> {
        let event = self.receiver.recv().await;
        if matches!(&event, Some(Err(_)) | None) {
            self.completed = true;
        }
        event
    }

    /// Returns a buffered item without waiting so HTTP shutdown can surface a terminal error.
    pub fn try_recv(&mut self) -> Option<Result<Event, BackendError>> {
        match self.receiver.try_recv() {
            Ok(event) => {
                if event.is_err() {
                    self.completed = true;
                }
                Some(event)
            }
            Err(mpsc::error::TryRecvError::Empty) => None,
            Err(mpsc::error::TryRecvError::Disconnected) => {
                self.completed = true;
                None
            }
        }
    }
}

impl<Event> Drop for SessionEventStream<Event> {
    fn drop(&mut self) {
        if !self.completed && self.commands.is_some() {
            ora_debug!(
                operation_id = self.operation_id.unwrap_or_default(),
                "stream dropped, sending cancel"
            );
            if let (Some(commands), Some(operation_id)) =
                (self.commands.as_ref(), self.operation_id)
            {
                let _ = commands.send(RuntimeCommand::Cancel { operation_id });
            }
        }
        if let Some(cleanup) = self.cleanup.take() {
            cleanup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SessionEventStream;
    use crate::BackendError;
    use tokio::sync::mpsc;

    /// Verifies a buffered terminal error is visible without waiting on `recv`.
    #[tokio::test]
    async fn try_recv_returns_a_buffered_error_without_waiting() {
        let (sender, receiver) = mpsc::channel::<Result<(), BackendError>>(1);
        let mut stream = SessionEventStream::with_cleanup(receiver, || {});
        sender
            .try_send(Err(BackendError::internal(
                "stream interrupted",
                std::io::Error::other("closed"),
            )))
            .expect("buffered error is queued");

        assert!(matches!(stream.try_recv(), Some(Err(_))));
        assert!(stream.try_recv().is_none());
    }
}
