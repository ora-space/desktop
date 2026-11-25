//! Configures a persisted session through the actor that reads its provider's frames.

use crate::command::{ConfigChangeRequest, RuntimeCommand};
use crate::config_changes::ConfigChangeRoute;
use crate::support::{agent_timed_out, runtime_unavailable};
use crate::{AgentRuntimeHost, AgentRuntimeManager, RuntimeError, SESSION_SETUP_TIMEOUT, start};
use agent_client_protocol_schema::v1::{SessionConfigId, SessionConfigOptionValue};
use ora_contracts::{SetSessionConfigRequest, SetSessionConfigResponse};
use tokio::sync::oneshot;
use tokio::time::timeout;

impl<H: AgentRuntimeHost> AgentRuntimeManager<H> {
    /// Applies one configuration option to a persisted session.
    pub async fn set_session_config(
        &self,
        request: SetSessionConfigRequest,
    ) -> Result<SetSessionConfigResponse, RuntimeError> {
        let config_id = SessionConfigId::new(request.config_id);
        let value = SessionConfigOptionValue::value_id(request.value);
        let session = self.find_session(&request.session_id)?;
        // The live actor asks, so that the answer takes its turn among the session's other frames,
        // and addresses the session it rebuilt rather than the one still in the row. A session with
        // no actor holds no rebuilt binding, so its row is authoritative.
        let route = match self.lookup_actor(&session.id)? {
            Some(handle) => {
                let (accepted, route) = oneshot::channel();
                let request = ConfigChangeRequest {
                    config_id: config_id.clone(),
                    value: value.clone(),
                    accepted,
                };
                handle
                    .commands
                    .send(RuntimeCommand::ChangeConfig(request))
                    .map_err(|_error| runtime_unavailable())?;
                route.await.map_err(|_error| runtime_unavailable())??
            }
            None => ConfigChangeRoute::Direct {
                agent_session_id: session.agent_session_id.clone(),
            },
        };
        let config_options = match route {
            ConfigChangeRoute::Direct { agent_session_id } => {
                start::request_config_option(
                    &self.inner.connections,
                    &session.agent_ref,
                    &agent_session_id,
                    &config_id,
                    &value,
                )
                .await?
            }
            // Giving up here does not withdraw the change: the agent may still make it, and the
            // actor adopts its answer then.
            ConfigChangeRoute::Sent(outcome) => timeout(SESSION_SETUP_TIMEOUT, outcome)
                .await
                .map_err(|_elapsed| agent_timed_out("agent configuration timed out"))?
                .map_err(|_lost| runtime_unavailable())??,
        };
        Ok(SetSessionConfigResponse { config_options })
    }
}
