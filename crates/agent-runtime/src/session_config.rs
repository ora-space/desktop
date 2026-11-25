//! Configures a persisted session by asking its provider directly.

use crate::command::RuntimeCommand;
use crate::support::runtime_unavailable;
use crate::{AgentRuntimeHost, AgentRuntimeManager, RuntimeError, start};
use agent_client_protocol_schema::v1::{SessionConfigId, SessionConfigOptionValue};
use ora_contracts::{SetSessionConfigRequest, SetSessionConfigResponse};
use tokio::sync::oneshot;

impl<H: AgentRuntimeHost> AgentRuntimeManager<H> {
    /// Applies one configuration option to a persisted session.
    pub async fn set_session_config(
        &self,
        request: SetSessionConfigRequest,
    ) -> Result<SetSessionConfigResponse, RuntimeError> {
        let config_id = SessionConfigId::new(request.config_id);
        let value = SessionConfigOptionValue::value_id(request.value);
        let session = self.find_session(&request.session_id)?;
        // The live actor is asked which provider session to address rather than reading the row,
        // because a session it rebuilt is not in the row until the transcript reaches it — writing
        // configuration to the row's identity would reach the provider being replaced. A session
        // with no actor holds no rebuilt binding, so its row is authoritative.
        let actor = self.lookup_actor(&session.id)?;
        let agent_session_id = match &actor {
            Some(handle) => {
                let (response, claimed) = oneshot::channel();
                handle
                    .commands
                    .send(RuntimeCommand::ClaimDirectProviderCall { response })
                    .map_err(|_error| runtime_unavailable())?;
                claimed.await.map_err(|_error| runtime_unavailable())?
            }
            None => session.agent_session_id.clone(),
        };
        // The provider request remains direct because it is independent of the actor's
        // serialized prompt/load stream; only the title-polling attempt needs standing down.
        let mut config_options = start::request_config_option(
            &self.inner.connections,
            &session.agent_ref,
            &agent_session_id,
            &config_id,
            &value,
        )
        .await?;
        if let Some(handle) = actor {
            let (response, adopted) = oneshot::channel();
            let command = RuntimeCommand::AdoptConfigOptions {
                agent_session_id,
                config_options: config_options.clone(),
                response,
            };
            // The provider has already changed, so an actor that ended meanwhile fails nothing.
            if handle.commands.send(command).is_ok()
                && let Ok(current) = adopted.await
            {
                config_options = current;
            }
        }
        Ok(SetSessionConfigResponse { config_options })
    }
}
