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
use tokio::sync::oneshot;
use tokio::time::timeout;

/// What a caller waiting for a configuration change is told.
pub(crate) type ConfigChangeOutcome = Result<Vec<SessionConfigOption>, RuntimeError>;

/// How a configuration change is on its way to the agent.
pub(crate) enum ConfigChangeRoute {
    /// The session holds no provider, so the caller asks the agent itself.
    Direct { agent_session_id: String },
    /// The actor asked, and tells the caller what came of it.
    Sent(oneshot::Receiver<ConfigChangeOutcome>),
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

/// Asks a provider session for one option on the stream that carries its other frames.
async fn ask(
    channel: &SessionChannel,
    agent_session_id: &str,
    config_id: &SessionConfigId,
    value: &SessionConfigOptionValue,
) -> Result<PendingSessionRequest<SetSessionConfigOptionResponse>, RuntimeError> {
    channel
        .connection
        .client
        .start_session_request(
            agent_client_protocol_schema::v1::SessionId::new(agent_session_id.to_string()),
            AGENT_METHOD_NAMES.session_set_config_option,
            &SetSessionConfigOptionRequest::new(
                agent_session_id.to_string(),
                config_id.clone(),
                value.clone(),
            ),
        )
        .await
        .map_err(map_acp_error)
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
    let pending = ask(channel, agent_session_id, config_id, value).await?;
    let read = timeout(SESSION_SETUP_TIMEOUT, async {
        loop {
            match channel.events.recv().await {
                Some(SessionEvent::Update(update, _)) => observe(channel, update),
                Some(SessionEvent::Response(response)) if pending.matches_response(&response) => {
                    return Some(response);
                }
                Some(event) => {
                    let client = channel.connection.client.clone();
                    settle_idle_event(&client, event).await;
                }
                None => return None,
            }
        }
    })
    .await;
    match read {
        Ok(Some(response)) => Ok(pending
            .finish(response)
            .map_err(map_acp_error)?
            .config_options),
        Ok(None) => Err(runtime_unavailable()),
        Err(_elapsed) => {
            channel.config_changes.unanswered.push(UnansweredChange {
                pending,
                caller: None,
            });
            Err(agent_timed_out("agent configuration timed out"))
        }
    }
}

impl<H: AgentRuntimeHost> RuntimeActor<H> {
    /// Asks the provider behind `channel` for one option, or leaves that to a caller when there is
    /// none.
    pub(crate) async fn change_config(
        &self,
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
            let _ = accepted.send(Ok(ConfigChangeRoute::Direct {
                agent_session_id: self.provider_session_id().to_string(),
            }));
            return;
        };
        match ask(channel, self.provider_session_id(), &config_id, &value).await {
            Ok(pending) => {
                let (caller, outcome) = oneshot::channel();
                channel.config_changes.unanswered.push(UnansweredChange {
                    pending,
                    caller: Some(caller),
                });
                let _ = accepted.send(Ok(ConfigChangeRoute::Sent(outcome)));
            }
            Err(error) => {
                let _ = accepted.send(Err(error));
            }
        }
    }

    /// Settles the change this response answers, if it answers one.
    ///
    /// Called from every place that reads a session's frames, at the answer's turn among them.
    pub(crate) fn settle_config_change(
        &mut self,
        channel: &mut SessionChannel,
        response: SessionResponse,
    ) {
        let Some(change) = channel.config_changes.answered_by(&response) else {
            return;
        };
        let UnansweredChange { pending, caller } = change;
        let outcome = match pending.finish(response) {
            Ok(answer) => {
                self.adopt_config_options(answer.config_options);
                Ok(self.reported_config_options.clone())
            }
            Err(error) => Err(map_acp_error(error)),
        };
        if let Some(caller) = caller {
            let _ = caller.send(outcome);
        }
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
