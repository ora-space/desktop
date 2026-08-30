//! Read facade over durable Effect state for the OpenCode MCP surface.
//!
//! The reconcile worker (`effect_worker`) mutates surfaces to match Desired State; this module is
//! the thin read side that folds the same durable rows plus a live Agent-availability fact into the
//! user-visible MCP Application State, without driving any reconcile. Splitting the read out of the
//! 2000-line worker keeps the read path — which the Settings UI polls — out of the reconcile
//! control-flow the worker owns.

use crate::error::BackendError;
use crate::plugin::PluginApi;
use ora_application::WorkspaceEffectService;
use ora_contracts::{GetMcpApplicationStateRequest, GetMcpApplicationStateResponse};
use ora_db::{RepositoryPool, SqliteEffectRepository};
use ora_domain::PluginId;

/// The consumer plugin id whose running generation serves the OpenCode MCP renderer.
///
/// The literal matches the consumer id the reconcile worker declares for the OpenCode surface, so
/// the read side and the reconcile side probe the same Agent process.
const OPENCODE_AGENT_PLUGIN_ID: &str = "official/ora-space.opencode";

/// Folds durable Effect state and a live Agent-availability fact into the MCP Application State.
///
/// The repository supplies the durable rows (desired-set, surface status, consumer statuses); the
/// plugin host supplies the one fact it alone knows — whether the OpenCode Agent process is
/// currently connected. The pure fold itself lives in the application service; this function wires
/// those two inputs together and maps the application error onto the backend's stable error so the
/// desktop command stays a thin adapter.
pub(crate) fn mcp_application_state(
    pool: &RepositoryPool,
    plugin_host: &PluginApi,
    request: GetMcpApplicationStateRequest,
) -> Result<GetMcpApplicationStateResponse, BackendError> {
    let service = WorkspaceEffectService::new(SqliteEffectRepository::new(pool.clone()));
    let plugin_id = PluginId::parse(OPENCODE_AGENT_PLUGIN_ID)
        .map_err(|error| BackendError::internal("invalid opencode plugin id", error))?;
    let agent_running = plugin_host.lifecycle.connection(&plugin_id).is_ok();
    service
        .mcp_application_state(request, agent_running)
        .map_err(|error| BackendError::internal("failed to read MCP application state", error))
}
