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
