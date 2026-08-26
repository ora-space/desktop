//! Maps Agent plugin Effect declarations and coordinates their runtime mutation boundary.

use ora_domain::PluginId;
use ora_effect::{
    ConsumerCoordination, ConsumerId, FilesystemSkillSurface, Generation, MaterializationFormat,
    SurfaceKey, SurfacePath,
};
use ora_plugin_runtime::{
    PluginEffectCoordination, PluginRegistration, PluginRuntime, PluginRuntimeError,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::path::Path;
use thiserror::Error;

pub(super) const WAIT_FOR_IDLE_METHOD: &str = "effect/waitForIdle";
pub(super) const RESTART_METHOD: &str = "effect/restart";

/// Reports an invalid registration or a failed Agent Effect coordination call.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub(crate) enum AgentEffectError {
    #[error("agent plugin Effect declaration is invalid: {0}")]
    InvalidDeclaration(String),
    #[error("agent plugin Effect IPC failed: {0}")]
    Ipc(String),
}

/// The result of asking an Agent plugin to establish its mutation barrier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WaitForIdleOutcome {
    Ready,
    WaitingForIdle,
}

/// Abstracts one IPC generation so the coordination protocol is testable without a real plugin.
trait AgentEffectRuntime {
    fn invoke(
        &self,
        method: &str,
        params: Value,
    ) -> impl Future<Output = Result<Value, String>> + Send;
}

impl AgentEffectRuntime for PluginRuntime {
    async fn invoke(&self, method: &str, params: Value) -> Result<Value, String> {
        PluginRuntime::invoke(self, method, params)
            .await
            .map_err(|error: PluginRuntimeError| error.to_string())
    }
}

/// Converts handshake declarations into host-owned descriptors for one Workspace.
pub(crate) fn registered_skill_surfaces(
    plugin_id: &PluginId,
    registration: &PluginRegistration,
) -> Result<Vec<FilesystemSkillSurface>, AgentEffectError> {
    registration
        .effect_surfaces
        .iter()
        .map(|surface| {
            let workspace_relative_path = SurfacePath::parse(&surface.workspace_relative_path)
                .map_err(|error| AgentEffectError::InvalidDeclaration(error.to_string()))?;
            let materialization_format =
                MaterializationFormat::named(surface.materialization_format.clone())
                    .map_err(|error| AgentEffectError::InvalidDeclaration(error.to_string()))?;
            if materialization_format != MaterializationFormat::skill_directory_v1() {
                return Err(AgentEffectError::InvalidDeclaration(format!(
                    "unsupported Skill materialization format {}",
                    surface.materialization_format
                )));
            }
            let coordination = match surface.coordination {
                PluginEffectCoordination::Uninterrupted => ConsumerCoordination::Uninterrupted,
                PluginEffectCoordination::WaitForIdleAndRestart => {
                    ConsumerCoordination::WaitForIdleAndRestart
                }
            };
            Ok(FilesystemSkillSurface {
                workspace_relative_path,
                materialization_format,
                // The canonical package identity is globally stable; plugins cannot impersonate
                // another consumer by selecting their own persisted consumer id.
                consumer: ConsumerId::new(plugin_id.canonical()),
                coordination,
            })
        })
        .collect()
}

/// Asks the plugin to wait for all affected Agent instances to become idle and hold a barrier.
#[allow(
    dead_code,
    reason = "the Effect worker will call this IPC seam once Agent plugins ship"
)]
pub(crate) async fn wait_for_idle(
    runtime: &PluginRuntime,
    surface_key: &SurfaceKey,
    workspace_root: &Path,
    relative_path: &SurfacePath,
) -> Result<WaitForIdleOutcome, AgentEffectError> {
    wait_for_idle_with(runtime, surface_key, workspace_root, relative_path).await
}

/// Restarts every affected Agent instance and releases the barrier for the applied generation.
#[allow(
    dead_code,
    reason = "the Effect worker will call this IPC seam once Agent plugins ship"
)]
pub(crate) async fn restart(
    runtime: &PluginRuntime,
    surface_key: &SurfaceKey,
    workspace_root: &Path,
    relative_path: &SurfacePath,
    generation: Generation,
) -> Result<(), AgentEffectError> {
    restart_with(
        runtime,
        surface_key,
        workspace_root,
        relative_path,
        generation,
    )
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SurfaceParams<'a> {
    surface_key: &'a str,
    workspace_root: &'a Path,
    relative_path: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RestartParams<'a> {
    surface_key: &'a str,
    workspace_root: &'a Path,
    relative_path: &'a str,
    generation: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum WaitState {
    Ready,
    WaitingForIdle,
}

#[derive(Deserialize)]
struct WaitResult {
    state: WaitState,
}

/// Runs the wait protocol against either the production IPC runtime or a test fake.
async fn wait_for_idle_with<Runtime: AgentEffectRuntime>(
    runtime: &Runtime,
    surface_key: &SurfaceKey,
    workspace_root: &Path,
    relative_path: &SurfacePath,
) -> Result<WaitForIdleOutcome, AgentEffectError> {
    let params = serde_json::to_value(SurfaceParams {
        surface_key: surface_key.as_str(),
        workspace_root,
        relative_path: relative_path.as_str(),
    })
    .map_err(|error| AgentEffectError::Ipc(error.to_string()))?;
    let value = runtime
        .invoke(WAIT_FOR_IDLE_METHOD, params)
        .await
        .map_err(AgentEffectError::Ipc)?;
    let result: WaitResult = serde_json::from_value(value)
        .map_err(|error| AgentEffectError::Ipc(format!("invalid wait result: {error}")))?;
    Ok(match result.state {
        WaitState::Ready => WaitForIdleOutcome::Ready,
        WaitState::WaitingForIdle => WaitForIdleOutcome::WaitingForIdle,
    })
}

/// Runs the restart protocol against either the production IPC runtime or a test fake.
async fn restart_with<Runtime: AgentEffectRuntime>(
    runtime: &Runtime,
    surface_key: &SurfaceKey,
    workspace_root: &Path,
    relative_path: &SurfacePath,
    generation: Generation,
) -> Result<(), AgentEffectError> {
    let params = serde_json::to_value(RestartParams {
        surface_key: surface_key.as_str(),
        workspace_root,
        relative_path: relative_path.as_str(),
        generation: generation.value(),
    })
    .map_err(|error| AgentEffectError::Ipc(error.to_string()))?;
    runtime
        .invoke(RESTART_METHOD, params)
        .await
        .map_err(AgentEffectError::Ipc)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_plugin_runtime::PluginEffectSurface;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::{Arc, Mutex, PoisonError};

    #[derive(Clone)]
    struct FakeRuntime {
        calls: Arc<Mutex<Vec<(String, Value)>>>,
        wait_result: Value,
    }

    impl AgentEffectRuntime for FakeRuntime {
        async fn invoke(&self, method: &str, params: Value) -> Result<Value, String> {
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((method.to_string(), params));
            if method == WAIT_FOR_IDLE_METHOD {
                Ok(self.wait_result.clone())
            } else {
                Ok(json!({}))
            }
        }
    }

    /// The host derives consumer identity from the package and rejects unsafe locators.
    #[test]
    fn maps_registered_locator_to_a_host_owned_surface() {
        let plugin_id = PluginId::new("official", "codex").expect("plugin id");
        let registration = PluginRegistration {
            effect_surfaces: vec![PluginEffectSurface {
                workspace_relative_path: ".codex/skills".to_string(),
                materialization_format: "skill_directory.v1".to_string(),
                coordination: PluginEffectCoordination::WaitForIdleAndRestart,
            }],
            ..PluginRegistration::default()
        };

        assert_eq!(
            registered_skill_surfaces(&plugin_id, &registration),
            Ok(vec![FilesystemSkillSurface {
                workspace_relative_path: SurfacePath::parse(".codex/skills").expect("surface path"),
                materialization_format: MaterializationFormat::skill_directory_v1(),
                consumer: ConsumerId::new("official/codex"),
                coordination: ConsumerCoordination::WaitForIdleAndRestart,
            }])
        );
    }

    /// A fake IPC generation proves waiting is non-destructive and restart carries the generation.
    #[tokio::test]
    async fn coordinates_wait_and_restart_without_a_real_agent_plugin() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let runtime = FakeRuntime {
            calls: calls.clone(),
            wait_result: json!({ "state": "waiting_for_idle" }),
        };
        let key = SurfaceKey::new("surface-1");
        let path = SurfacePath::parse(".codex/skills").expect("surface path");
        let root = Path::new("/workspace");

        assert_eq!(
            wait_for_idle_with(&runtime, &key, root, &path).await,
            Ok(WaitForIdleOutcome::WaitingForIdle)
        );
        restart_with(&runtime, &key, root, &path, Generation::new(7))
            .await
            .expect("restart");
        assert_eq!(
            calls.lock().unwrap_or_else(PoisonError::into_inner).clone(),
            vec![
                (
                    WAIT_FOR_IDLE_METHOD.to_string(),
                    json!({
                        "surfaceKey": "surface-1",
                        "workspaceRoot": "/workspace",
                        "relativePath": ".codex/skills"
                    })
                ),
                (
                    RESTART_METHOD.to_string(),
                    json!({
                        "surfaceKey": "surface-1",
                        "workspaceRoot": "/workspace",
                        "relativePath": ".codex/skills",
                        "generation": 7
                    })
                )
            ]
        );
    }
}
