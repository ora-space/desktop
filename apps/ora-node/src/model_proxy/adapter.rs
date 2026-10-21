//! Host-owned grant conversion is generic except for the explicit undeclared OpenCode route.

use super::{Grant, ModelAccessError, Protocol, opencode};
use ora_plugin_protocol::{
    AgentBoundModel, AgentModelConfiguration, AgentModelContext, AgentModelProtocol,
};
use std::{collections::BTreeMap, path::Path};

/// A declaration selects the generic live startup RPC; absence retains existing OpenCode behavior.
pub(super) enum ModelAdapter {
    LegacyOpenCode,
    Declared(Box<AgentModelConfiguration>),
}

/// The owning ModelAccess retains the private directory until all plugin processes have ended.
pub(super) struct AdapterState {
    pub(super) environment: BTreeMap<String, String>,
    pub(super) home: tempfile::TempDir,
    pub(super) model_context: Option<AgentModelContext>,
}

impl ModelAdapter {
    /// Checks frozen metadata before exposing authority or creating any adapter-specific state.
    pub(super) fn prepare(
        self,
        grant: &Grant,
        ca: &Path,
        node_home: &Path,
    ) -> Result<AdapterState, ModelAccessError> {
        match self {
            Self::LegacyOpenCode => {
                let (environment, home) = opencode::environment(grant, ca, node_home)?;
                Ok(AdapterState {
                    environment,
                    home,
                    model_context: None,
                })
            }
            Self::Declared(configuration) => {
                let configuration = *configuration;
                let protocol = match grant.protocol {
                    Protocol::OpenaiCompletions => AgentModelProtocol::OpenaiCompletions,
                    Protocol::AnthropicMessages => AgentModelProtocol::AnthropicMessages,
                };
                let model = AgentBoundModel {
                    id: grant.model.id.clone(),
                    name: grant.model.name.clone(),
                    context_window: grant.model.context_window,
                    max_tokens: grant.model.max_tokens,
                };
                if configuration.protocol != protocol || configuration.model != model {
                    return Err(ModelAccessError("model_binding_mismatch"));
                }
                let (environment, home) = isolated_environment(node_home)?;
                Ok(AdapterState {
                    environment,
                    home,
                    model_context: Some(AgentModelContext {
                        configuration,
                        proxy_base_url: grant.proxy_base_url.clone(),
                        token: grant.token.clone(),
                    }),
                })
            }
        }
    }
}

/// A declared adapter gets private state but no CLI routing or token in its environment.
fn isolated_environment(
    node_home: &Path,
) -> Result<(BTreeMap<String, String>, tempfile::TempDir), ModelAccessError> {
    let root = node_home.join("model-runtime");
    ora_utils::path::create_directories_without_symlinks(&root)
        .map_err(|_| ModelAccessError("model_runtime_state_unavailable"))?;
    let home = tempfile::Builder::new()
        .prefix("agent-")
        .tempdir_in(root)
        .map_err(|_| ModelAccessError("model_runtime_state_unavailable"))?;
    let environment = [
        ("HOME", home.path().to_path_buf()),
        ("XDG_CONFIG_HOME", home.path().join("config")),
        ("XDG_CACHE_HOME", home.path().join("cache")),
        ("XDG_DATA_HOME", home.path().join("data")),
        ("XDG_STATE_HOME", home.path().join("state")),
    ]
    .into_iter()
    .map(|(key, path)| (key.to_string(), path.to_string_lossy().into_owned()))
    .collect();
    Ok((environment, home))
}
