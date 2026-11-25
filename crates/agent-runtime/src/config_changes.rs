//! Orders configuration answers with the other frames of their session and settles the changes that
//! are still waiting for one when their provider session goes.

use crate::command::ConfigChangeRequest;
use crate::events::settle_idle_event;
use crate::routing::{SessionChannel, SessionEvent};
use crate::support::{agent_timed_out, map_acp_error, runtime_unavailable};
use crate::{AgentRuntimeHost, RuntimeActor, RuntimeError, SESSION_SETUP_TIMEOUT};
use agent_client_protocol_schema::v1::{
    AGENT_METHOD_NAMES, ConfigOptionUpdate, SessionConfigId, SessionConfigOption,
    SessionConfigOptionValue, SessionNotification, SessionUpdate, SetSessionConfigOptionRequest,
    SetSessionConfigOptionResponse,
};
use ora_acp::{PendingSessionRequest, SessionResponse};
use std::future::Future;
use std::pin::Pin;
use tokio::sync::oneshot::{self, error::TryRecvError};
use tokio::time::timeout;

/// What a caller waiting for a configuration change is told.
pub(crate) type ConfigChangeOutcome = Result<Vec<SessionConfigOption>, RuntimeError>;

/// Writes one request the actor did not wait for.
pub(crate) type ConfigChangeWrite = Pin<Box<dyn Future<Output = Result<(), RuntimeError>> + Send>>;

/// How a configuration change is on its way to the agent.
pub(crate) enum ConfigChangeRoute {
    /// The session holds no provider, so the caller asks the agent itself.
    ///
    /// Attaching waits until `_finished` is dropped: an answer that raced it could not be placed
    /// among the frames of the session being attached.
    Direct {
        agent_session_id: String,
        _finished: Option<oneshot::Sender<()>>,
    },
    /// The actor waits for the answer, and tells the caller what came of it.
    Asked {
        /// What is left of writing the request, when the agent was not reading.
        write: Option<ConfigChangeWrite>,
        outcome: oneshot::Receiver<ConfigChangeOutcome>,
    },
}

/// The changes one provider session was asked for and has not answered.
///
/// They live with its channel because only that channel carries their answers. Dropping it is what
/// fails them, so a change the host gave up on itself has to be accepted before.
#[derive(Default)]
pub(crate) struct ConfigChanges {
    unanswered: Vec<UnansweredChange>,
}

/// One change, and the caller waiting for it unless it gave up.
struct UnansweredChange {
    pending: PendingSessionRequest<SetSessionConfigOptionResponse>,
    caller: Option<oneshot::Sender<ConfigChangeOutcome>>,
    /// Told once the request is written, when that was left to the caller.
    written: Option<oneshot::Receiver<()>>,
}

impl ConfigChanges {
    /// Reports every change as made, although its answer can no longer arrive.
    ///
    /// For a provider session the host gave up: the agent was asked, and nothing suggests that it
    /// refused. A session lost to its agent must not come here.
    pub(crate) fn accept(&mut self, options: &[SessionConfigOption]) {
        for change in self.unanswered.drain(..) {
            change.tell(Ok(options.to_vec()));
        }
    }

    /// Forgets the changes whose callers gave up before the agent was asked.
    fn forget_unasked(&mut self) {
        self.unanswered
            .retain_mut(|change| match change.written.as_mut() {
                Some(written) => match written.try_recv() {
                    Ok(()) => {
                        change.written = None;
                        true
                    }
                    Err(TryRecvError::Empty) => true,
                    Err(TryRecvError::Closed) => false,
                },
                None => true,
            });
    }

    /// Takes out the change this response answers.
    fn answered_by(&mut self, response: &SessionResponse) -> Option<UnansweredChange> {
        let index = self
            .unanswered
            .iter()
            .position(|change| change.pending.matches_response(response))?;
        Some(self.unanswered.remove(index))
    }
}

impl UnansweredChange {
    /// Tells the caller what came of the change.
    fn tell(self, outcome: ConfigChangeOutcome) {
        if let Some(caller) = self.caller {
            let _ = caller.send(outcome);
        }
    }
}

/// Registers a change on the stream that carries a provider session's other frames, and returns what
/// is left to write.
fn prepare(
    channel: &SessionChannel,
    agent_session_id: &str,
    config_id: &SessionConfigId,
    value: &SessionConfigOptionValue,
) -> (
    PendingSessionRequest<SetSessionConfigOptionResponse>,
    ConfigChangeWrite,
) {
    let (pending, write) = channel.connection.client.prepare_session_request(
        agent_client_protocol_schema::v1::SessionId::new(agent_session_id.to_string()),
        AGENT_METHOD_NAMES.session_set_config_option,
        &SetSessionConfigOptionRequest::new(
            agent_session_id.to_string(),
            config_id.clone(),
            value.clone(),
        ),
    );
    (
        pending,
        Box::pin(async move { write.await.map_err(map_acp_error) }),
    )
}

/// Asks a provider session being set up for one option, reading its frames up to the answer.
///
/// `observe` is given every update that came before the answer, in order. An answer that comes too
/// late for this setup is adopted by whoever reads the channel then.
pub(crate) async fn ask_during_setup(
    channel: &mut SessionChannel,
    agent_session_id: &str,
    config_id: &SessionConfigId,
    value: &SessionConfigOptionValue,
    mut observe: impl FnMut(&mut SessionChannel, SessionNotification),
) -> ConfigChangeOutcome {
    let (pending, write) = prepare(channel, agent_session_id, config_id, value);
    let read = timeout(SESSION_SETUP_TIMEOUT, async {
        write.await?;
        loop {
            match channel.events.recv().await {
                Some(SessionEvent::Update(update, _)) => observe(channel, update),
                Some(SessionEvent::Response(response)) if pending.matches_response(&response) => {
                    return Ok(response);
                }
                Some(event) => {
                    let client = channel.connection.client.clone();
                    settle_idle_event(&client, event).await;
                }
                None => return Err(runtime_unavailable()),
            }
        }
    })
    .await;
    match read {
        Ok(Ok(response)) => Ok(pending
            .finish(response)
            .map_err(map_acp_error)?
            .config_options),
        Ok(Err(error)) => Err(error),
        Err(_elapsed) => {
            channel.config_changes.unanswered.push(UnansweredChange {
                pending,
                caller: None,
                written: None,
            });
            Err(agent_timed_out("agent configuration timed out"))
        }
    }
}

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Asks the provider behind `channel` for one option, or leaves that to a caller when there is
    /// none.
    pub(crate) async fn change_config(
        &mut self,
        channel: Option<&mut SessionChannel>,
        request: ConfigChangeRequest,
    ) {
        let ConfigChangeRequest {
            config_id,
            value,
            accepted,
        } = request;
        // A caller that gave up before the actor got here never asked for anything.
        if accepted.is_closed() {
            return;
        }
        let Some(channel) = channel else {
            let (finished, done) = oneshot::channel();
            self.direct_config_changes.push(done);
            let _ = accepted.send(Ok(ConfigChangeRoute::Direct {
                agent_session_id: self.provider_session_id().to_string(),
                _finished: Some(finished),
            }));
            return;
        };
        let (pending, mut write) = prepare(channel, self.provider_session_id(), &config_id, &value);
        // Written here while the agent reads, which keeps changes in the order they were asked for.
        // An agent that does not must hold up its caller alone: this actor still has to stop.
        let write = tokio::select! {
            biased;
            written = &mut write => match written {
                Ok(()) => None,
                Err(error) => {
                    let _ = accepted.send(Err(error));
                    return;
                }
            },
            () = std::future::ready(()) => Some(write),
        };
        let (write, written) = match write {
            Some(write) => {
                let (report, written) = oneshot::channel();
                let write: ConfigChangeWrite = Box::pin(async move {
                    write.await?;
                    let _ = report.send(());
                    Ok(())
                });
                (Some(write), Some(written))
            }
            None => (None, None),
        };
        channel.config_changes.forget_unasked();
        let (caller, outcome) = oneshot::channel();
        channel.config_changes.unanswered.push(UnansweredChange {
            pending,
            caller: Some(caller),
            written,
        });
        let _ = accepted.send(Ok(ConfigChangeRoute::Asked { write, outcome }));
    }

    /// Waits for the changes whose callers are asking a session that holds no provider.
    ///
    /// Each caller gives up after its own timeout.
    pub(crate) async fn finish_direct_changes(&mut self) {
        for finished in std::mem::take(&mut self.direct_config_changes) {
            let _ = finished.await;
        }
    }

    /// Settles the change this response answers, if it answers one.
    ///
    /// Called from every place that reads a session's frames, at the answer's turn among them. Returns
    /// the options to show when its caller is gone, because nobody else tells the views then.
    pub(crate) fn settle_config_change(
        &mut self,
        channel: &mut SessionChannel,
        response: SessionResponse,
    ) -> Option<SessionUpdate> {
        let UnansweredChange {
            pending, caller, ..
        } = channel.config_changes.answered_by(&response)?;
        let outcome = match pending.finish(response) {
            Ok(answer) => {
                self.adopt_config_options(answer.config_options);
                Ok(self.reported_config_options.clone())
            }
            Err(error) => Err(map_acp_error(error)),
        };
        let untold = match caller {
            Some(caller) => caller.send(outcome).err()?,
            None => outcome,
        };
        let options = untold.ok()?;
        Some(SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(
            options,
        )))
    }

    /// Takes the options one answer carries, as if the agent had reported them.
    pub(crate) fn adopt_config_options(&mut self, config_options: Vec<SessionConfigOption>) {
        self.observe_session_update(&SessionUpdate::ConfigOptionUpdate(ConfigOptionUpdate::new(
            config_options,
        )));
    }

    /// Reports the unanswered changes of a session the host gives up as made.
    pub(crate) fn accept_changes_on(&self, channel: &mut SessionChannel) {
        channel.config_changes.accept(&self.reported_config_options);
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigChanges, UnansweredChange};
    use ora_acp::{AcpError, AcpPeer, AcpTransport};
    use pretty_assertions::assert_eq;
    use serde_json::{Value, json};
    use tokio::sync::{mpsc, oneshot};

    /// Takes every frame, because nothing here is answered.
    struct Sink;

    impl AcpTransport for Sink {
        async fn send(&self, _message: Value) -> Result<(), AcpError> {
            Ok(())
        }
    }

    /// Only a change that can no longer be written is forgotten, however often that is looked at.
    #[tokio::test]
    async fn only_changes_that_were_never_written_are_forgotten() {
        let (_inbound, messages) = mpsc::unbounded_channel();
        let peer = AcpPeer::spawn(messages, Sink);
        let mut changes = ConfigChanges::default();
        let mut expect = |written| {
            let (pending, _write) = peer.client.prepare_session_request(
                agent_client_protocol_schema::v1::SessionId::new("session-1"),
                "session/set_config_option",
                &json!({}),
            );
            changes.unanswered.push(UnansweredChange {
                pending,
                caller: None,
                written,
            });
        };
        let (done, written) = oneshot::channel();
        done.send(()).unwrap();
        let (_waiting, unwritten) = oneshot::channel();
        let (gone, given_up) = oneshot::channel::<()>();
        drop(gone);
        expect(Some(written));
        expect(Some(unwritten));
        expect(Some(given_up));
        expect(None);

        for _ in 0..2 {
            changes.forget_unasked();
            assert_eq!(changes.unanswered.len(), 3);
        }
    }
}
