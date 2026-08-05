use super::RuntimeCommand;
use super::routing::{SessionControl, SessionEvent};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;

const MAX_EVENTS_BEFORE_COMMAND_POLL: usize = 32;

enum DeferredControl {
    Message(SessionControl),
    ChannelClosed,
}

impl DeferredControl {
    /// Converts a deferred control state back into the actor's input stream.
    fn into_input(self) -> ActiveInput {
        match self {
            Self::Message(control) => ActiveInput::Control(control),
            Self::ChannelClosed => ActiveInput::ControlsClosed,
        }
    }
}

/// Represents one ready source consumed by an active load or prompt actor.
pub(super) enum ActiveInput {
    Event(SessionEvent),
    EventsClosed,
    Control(SessionControl),
    ControlsClosed,
    Command(RuntimeCommand),
    CommandsClosed,
}

/// Preserves terminal-control ordering while bounding command latency during event bursts.
#[derive(Default)]
pub(super) struct ActiveInputState {
    deferred_control: Option<DeferredControl>,
    events_since_command_poll: usize,
}

impl ActiveInputState {
    /// Receives active actor input without letting a live event stream starve commands.
    ///
    /// Connection and overflow controls are emitted only after their route is detached. If a
    /// control wins the select, every event already queued ahead of it is therefore safe to drain
    /// before exposing the terminal control to the actor.
    pub(super) async fn recv(
        &mut self,
        events: &mut mpsc::Receiver<SessionEvent>,
        controls: &mut mpsc::UnboundedReceiver<SessionControl>,
        commands: &mut mpsc::UnboundedReceiver<RuntimeCommand>,
    ) -> ActiveInput {
        if let Some(control) = self.deferred_control.take() {
            return match events.try_recv() {
                Ok(event) => {
                    self.deferred_control = Some(control);
                    ActiveInput::Event(event)
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => control.into_input(),
            };
        }

        if self.events_since_command_poll >= MAX_EVENTS_BEFORE_COMMAND_POLL {
            self.events_since_command_poll = 0;
            match commands.try_recv() {
                Ok(command) => return ActiveInput::Command(command),
                Err(TryRecvError::Disconnected) => return ActiveInput::CommandsClosed,
                Err(TryRecvError::Empty) => {}
            }
        }

        tokio::select! {
            event = events.recv() => match event {
                Some(event) => {
                    self.events_since_command_poll =
                        self.events_since_command_poll.saturating_add(1);
                    ActiveInput::Event(event)
                }
                None => ActiveInput::EventsClosed,
            },
            control = controls.recv() => {
                self.events_since_command_poll = 0;
                let control = match control {
                    Some(control) => DeferredControl::Message(control),
                    None => DeferredControl::ChannelClosed,
                };
                match events.try_recv() {
                    Ok(event) => {
                        self.deferred_control = Some(control);
                        ActiveInput::Event(event)
                    }
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => control.into_input(),
                }
            },
            command = commands.recv() => {
                self.events_since_command_poll = 0;
                match command {
                    Some(command) => ActiveInput::Command(command),
                    None => ActiveInput::CommandsClosed,
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ActiveInput, ActiveInputState, DeferredControl, MAX_EVENTS_BEFORE_COMMAND_POLL};
    use crate::agent_runtime::RuntimeCommand;
    use crate::agent_runtime::routing::{SessionControl, SessionEvent};
    use ora_contracts::acp::notification::SessionNotification;
    use ora_contracts::acp::session::{SessionInfoUpdate, SessionUpdate};
    use tokio::sync::mpsc;

    /// Verifies a terminal control cannot overtake events already accepted by the session FIFO.
    #[tokio::test]
    async fn drains_queued_events_before_a_terminal_control() {
        let (events_sender, mut events) = mpsc::channel(/*buffer*/ 1);
        let (_controls_sender, mut controls) = mpsc::unbounded_channel();
        let (_commands_sender, mut commands) = mpsc::unbounded_channel();
        events_sender
            .send(SessionEvent::Update(SessionNotification::new(
                "session-1",
                SessionUpdate::SessionInfoUpdate(SessionInfoUpdate::new()),
            )))
            .await
            .expect("queue session event");
        let mut state = ActiveInputState {
            deferred_control: Some(DeferredControl::Message(SessionControl::QueueOverflow)),
            events_since_command_poll: 0,
        };

        assert!(matches!(
            state.recv(&mut events, &mut controls, &mut commands).await,
            ActiveInput::Event(SessionEvent::Update(_))
        ));
        assert!(matches!(
            state.recv(&mut events, &mut controls, &mut commands).await,
            ActiveInput::Control(SessionControl::QueueOverflow)
        ));
    }

    /// Verifies a ready command is forced through after a bounded event burst.
    #[tokio::test]
    async fn polls_commands_after_a_bounded_event_burst() {
        let (events_sender, mut events) =
            mpsc::channel(/*buffer*/ MAX_EVENTS_BEFORE_COMMAND_POLL + 1);
        let (_controls_sender, mut controls) = mpsc::unbounded_channel();
        let (commands_sender, mut commands) = mpsc::unbounded_channel();
        for index in 0..=MAX_EVENTS_BEFORE_COMMAND_POLL {
            events_sender
                .send(SessionEvent::Update(SessionNotification::new(
                    "session-1",
                    SessionUpdate::SessionInfoUpdate(
                        SessionInfoUpdate::new().title(format!("Update {index}")),
                    ),
                )))
                .await
                .expect("queue session event");
        }
        let mut state = ActiveInputState::default();
        for _ in 0..MAX_EVENTS_BEFORE_COMMAND_POLL {
            assert!(matches!(
                state.recv(&mut events, &mut controls, &mut commands).await,
                ActiveInput::Event(SessionEvent::Update(_))
            ));
        }
        commands_sender
            .send(RuntimeCommand::Cancel { operation_id: 7 })
            .expect("queue cancel command");

        assert!(matches!(
            state.recv(&mut events, &mut controls, &mut commands).await,
            ActiveInput::Command(RuntimeCommand::Cancel { operation_id: 7 })
        ));
    }
}
