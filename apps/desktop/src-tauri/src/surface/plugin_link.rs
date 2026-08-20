//! Host-to-plugin notifications about surface sessions (`ui/surfaceOpened` / `ui/surfaceClosed`)
//! and the on-demand process start triggered by opening a surface.

use crate::surface::gateway::{GatewayFailure, SurfaceConnection, SurfacePluginGateway};
use ora_domain::PluginId;
use ora_logging::{ora_debug, ora_warn};
use ora_plugin_lifecycle::ConnectionError;
use ora_surface::{SurfaceRecord, SurfaceRegistry, SurfaceState};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

const SURFACE_OPENED_METHOD: &str = "ui/surfaceOpened";
const SURFACE_CLOSED_METHOD: &str = "ui/surfaceClosed";

/// How long an open waits for the plugin process before giving up on the notification.
pub const PROCESS_START_WAIT: Duration = Duration::from_secs(15);

/// Builds the `{ surfaceId, instanceId, generation }` params both notifications carry.
pub fn session_params(record: &SurfaceRecord, generation: u64) -> Value {
    json!({
        "surfaceId": record.definition.id.surface_id.as_str(),
        "instanceId": record.instance.value(),
        "generation": generation,
    })
}

/// Sends `ui/surfaceOpened` for a freshly mounted instance and starts the process if needed.
///
/// A running process is notified directly. A stopped or failed one is started in a spawned
/// task and, once running, receives `ui/surfaceOpened` for every open instance of the plugin
/// so it sees the complete session view. A process that is already starting only needs the
/// new instance replayed, because the task that started it replays the rest. Startup failures
/// are logged and never affect the surface: remote sites do not depend on the process.
pub fn announce_opened<G: SurfacePluginGateway>(
    gateway: &G,
    registry: &Arc<SurfaceRegistry>,
    record: &SurfaceRecord,
) -> Option<ProcessStart<G::Connection>> {
    match gateway.connection(&record.definition.id.plugin_id) {
        Ok(connection) => {
            let params = session_params(record, connection.generation().0);
            Some(ProcessStart::Notify { connection, params })
        }
        Err(GatewayFailure::Connection(ConnectionError::NotReady)) => Some(ProcessStart::Await {
            plugin_id: record.definition.id.plugin_id.clone(),
            replay: Replay::Only(record.instance.value()),
            registry: registry.clone(),
        }),
        Err(GatewayFailure::Connection(
            ConnectionError::NotRunning | ConnectionError::Failed(_) | ConnectionError::Timeout,
        )) => Some(ProcessStart::Await {
            plugin_id: record.definition.id.plugin_id.clone(),
            replay: Replay::All,
            registry: registry.clone(),
        }),
        Err(GatewayFailure::Connection(ConnectionError::NotFound | ConnectionError::Disabled))
        | Err(GatewayFailure::Other(_)) => {
            ora_debug!(
                message = "plugin process not started for surface",
                plugin_id = %record.definition.id.plugin_id,
                instance = record.instance.value(),
            );
            None
        }
    }
}

/// Which instances to announce once the process is running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Replay {
    All,
    Only(u64),
}

/// The asynchronous follow-up of `announce_opened`, executed by the caller's spawner.
pub enum ProcessStart<C> {
    Notify {
        connection: C,
        params: Value,
    },
    Await {
        plugin_id: PluginId,
        replay: Replay,
        registry: Arc<SurfaceRegistry>,
    },
}

impl<C: SurfaceConnection> ProcessStart<C> {
    /// Runs the follow-up to completion; all failures are logged, none propagate.
    pub async fn run<G: SurfacePluginGateway<Connection = C>>(self, gateway: &G) {
        match self {
            Self::Notify { connection, params } => {
                notify(&connection, SURFACE_OPENED_METHOD, params).await
            }
            Self::Await {
                plugin_id,
                replay,
                registry,
            } => match gateway.ensure_running(&plugin_id, PROCESS_START_WAIT).await {
                Ok(connection) => {
                    let generation = connection.generation().0;
                    for record in registry.instances_of(&plugin_id) {
                        let mounted = matches!(
                            record.state,
                            SurfaceState::Embedded { .. } | SurfaceState::Windowed { .. }
                        );
                        let wanted = match replay {
                            Replay::All => true,
                            Replay::Only(instance) => record.instance.value() == instance,
                        };
                        if mounted && wanted {
                            notify(
                                &connection,
                                SURFACE_OPENED_METHOD,
                                session_params(&record, generation),
                            )
                            .await;
                        }
                    }
                }
                Err(error) => ora_warn!(
                    message = "plugin process did not start for surface; surface stays usable",
                    plugin_id = %plugin_id,
                    error = %error,
                ),
            },
        }
    }
}

/// Sends `ui/surfaceClosed` when the process is running; a stopped process is not started for it.
pub async fn announce_closed<G: SurfacePluginGateway>(gateway: &G, record: SurfaceRecord) {
    if let Ok(connection) = gateway.connection(&record.definition.id.plugin_id) {
        let params = session_params(&record, connection.generation().0);
        notify(&connection, SURFACE_CLOSED_METHOD, params).await;
    }
}

/// Sends one notification and logs a failure without retrying.
async fn notify<C: SurfaceConnection>(connection: &C, method: &str, params: Value) {
    if let Err(error) = connection.notify(method, params).await {
        ora_warn!(
            message = "surface notification to plugin failed",
            method,
            generation = connection.generation().0,
            error = %error,
        );
    }
}
