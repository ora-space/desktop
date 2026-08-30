//! Maps Agent plugin Effect declarations and coordinates their runtime mutation boundary.

use ora_domain::PluginId;
use ora_effect::{
    ConsumerCoordination, ConsumerId, DesiredMcpState, Digest, FilesystemSkillSurface, Generation,
    MaterializationFormat, RenderedMcpFile, SurfaceKey, SurfacePath,
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
/// The method a renderer plugin serves to produce the complete OpenCode MCP file.
pub(super) const RENDER_MCP_METHOD: &str = "agent_mcp_v1/render";

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
pub(crate) async fn wait_for_idle(
    runtime: &PluginRuntime,
    surface_key: &SurfaceKey,
    workspace_root: &Path,
    relative_path: &SurfacePath,
) -> Result<WaitForIdleOutcome, AgentEffectError> {
    wait_for_idle_with(runtime, surface_key, workspace_root, relative_path).await
}

/// Restarts every affected Agent instance and releases the barrier for the applied generation.
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

/// Request the host sends to ask a renderer plugin for the complete OpenCode MCP file.
///
/// Carries only environment-variable references and static recipe text — never a Setting value — so
/// a renderer cannot leak a key it was never handed. The camelCase field names are the stable IPC
/// contract the TypeScript renderer implements; they are deliberately distinct from the snake-case
/// shape [`DesiredMcpState`] persists in, so the wire form never crosses that boundary.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RenderMcpRequest<'a> {
    servers: Vec<McpServerRef<'a>>,
}

/// One plaintext-free MCP server in a render request.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct McpServerRef<'a> {
    namespace: &'a str,
    identifier: &'a str,
    version: &'a str,
    definition_digest: &'a str,
    revision: u64,
    url: &'a str,
    headers: Vec<McpHttpHeaderRef<'a>>,
}

/// One header whose value the Agent resolves at start from a named environment variable.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct McpHttpHeaderRef<'a> {
    name: &'a str,
    env_var: &'a str,
    prefix: &'a str,
    suffix: &'a str,
}

/// Result a renderer plugin returns: the complete-file bytes and the digest the host rechecks.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenderMcpResult {
    bytes: String,
    digest: String,
}

/// Renders the complete OpenCode MCP file from a plaintext-free set against an IPC runtime.
///
/// The host never trusts the plugin's digest: it recomputes the content digest over the returned
/// bytes and rejects a mismatch, so a malformed or tampered render cannot write a file whose marker
/// would later claim a different identity. The verified digest is the one the Effect reconcile path
/// stamps into the file's ownership marker.
async fn render_mcp_complete_file_with<Runtime: AgentEffectRuntime>(
    runtime: &Runtime,
    desired: &[DesiredMcpState],
) -> Result<RenderedMcpFile, AgentEffectError> {
    let servers = desired
        .iter()
        .map(|state| McpServerRef {
            namespace: state.namespace.as_ref(),
            identifier: state.identifier.as_str(),
            version: state.version.as_str(),
            definition_digest: state.definition_digest.as_str(),
            revision: state.revision,
            url: state.transport.url.as_str(),
            headers: state
                .transport
                .headers
                .iter()
                .map(|header| McpHttpHeaderRef {
                    name: header.name.as_str(),
                    env_var: header.env_var.as_str(),
                    prefix: header.prefix.as_str(),
                    suffix: header.suffix.as_str(),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let params = serde_json::to_value(RenderMcpRequest { servers })
        .map_err(|error| AgentEffectError::Ipc(error.to_string()))?;
    let value = runtime
        .invoke(RENDER_MCP_METHOD, params)
        .await
        .map_err(AgentEffectError::Ipc)?;
    let result: RenderMcpResult = serde_json::from_value(value)
        .map_err(|error| AgentEffectError::Ipc(format!("invalid render result: {error}")))?;
    let computed = Digest::sha256(result.bytes.as_bytes());
    if computed.as_str() != result.digest {
        return Err(AgentEffectError::Ipc(
            "rendered file digest does not match its bytes".to_string(),
        ));
    }
    Ok(RenderedMcpFile {
        bytes: result.bytes,
        digest: computed,
    })
}

/// Renders the complete OpenCode MCP file through the production plugin runtime.
///
/// Mirrors [`wait_for_idle`] and [`restart`]: a concrete, non-generic `pub(crate)` entry point the
/// coordinator calls, delegating to the generic `_with` helper that the test fake also satisfies.
pub(crate) async fn render_mcp_complete_file(
    runtime: &PluginRuntime,
    desired: &[DesiredMcpState],
) -> Result<RenderedMcpFile, AgentEffectError> {
    render_mcp_complete_file_with(runtime, desired).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use ora_domain::Namespace;
    use ora_effect::{McpHttpHeaderEffect, McpHttpTransportEffect};
    use ora_plugin_runtime::PluginEffectSurface;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::{Arc, Mutex, PoisonError};

    #[derive(Clone)]
    struct FakeRuntime {
        calls: Arc<Mutex<Vec<(String, Value)>>>,
        wait_result: Value,
        render_result: Value,
    }

    impl AgentEffectRuntime for FakeRuntime {
        async fn invoke(&self, method: &str, params: Value) -> Result<Value, String> {
            self.calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((method.to_string(), params));
            if method == WAIT_FOR_IDLE_METHOD {
                Ok(self.wait_result.clone())
            } else if method == RENDER_MCP_METHOD {
                Ok(self.render_result.clone())
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
            render_result: json!({}),
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

    /// Builds the Tavily-shaped desired state used by render-contract tests.
    fn tavily_desired() -> DesiredMcpState {
        DesiredMcpState {
            namespace: Namespace::new("official").expect("namespace"),
            identifier: "ora-space.tavily-search".to_string(),
            version: "1.0.0".to_string(),
            definition_digest: "deadbeef".to_string(),
            revision: 1,
            transport: McpHttpTransportEffect {
                url: "https://mcp.tavily.com/mcp".to_string(),
                headers: vec![McpHttpHeaderEffect {
                    name: "Authorization".to_string(),
                    env_var: "ORA_MCP_OFFICIAL_ORA_SPACE_TAVILY_SEARCH_APIKEY_0".to_string(),
                    prefix: "Bearer ".to_string(),
                    suffix: String::new(),
                }],
            },
        }
    }

    /// A fake agent renders the complete OpenCode MCP file from a plaintext-free desired set: the
    /// host sends only env-var references (never a Setting value), and it verifies the returned
    /// digest against the returned bytes before trusting either.
    #[tokio::test]
    async fn renders_the_complete_mcp_file_from_a_plaintext_free_desired_set() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let bytes = r#"{"$schema":"https://opencode.ai/config.json","mcp":{"ora__ora-space__tavily-search":{"type":"remote","url":"https://mcp.tavily.com/mcp","enabled":true,"headers":{"Authorization":"Bearer {env:ORA_MCP_OFFICIAL_ORA_SPACE_TAVILY_SEARCH_APIKEY_0}"}}}}"#.to_string();
        let digest = Digest::sha256(bytes.as_bytes());
        let runtime = FakeRuntime {
            calls: calls.clone(),
            wait_result: json!({}),
            render_result: json!({ "bytes": bytes.clone(), "digest": digest.as_str() }),
        };
        let desired = tavily_desired();

        let rendered = render_mcp_complete_file_with(&runtime, std::slice::from_ref(&desired))
            .await
            .expect("render");

        assert_eq!(
            calls.lock().unwrap_or_else(PoisonError::into_inner).clone(),
            vec![(
                RENDER_MCP_METHOD.to_string(),
                json!({
                    "servers": [{
                        "namespace": "official",
                        "identifier": "ora-space.tavily-search",
                        "version": "1.0.0",
                        "definitionDigest": "deadbeef",
                        "revision": 1,
                        "url": "https://mcp.tavily.com/mcp",
                        "headers": [{
                            "name": "Authorization",
                            "envVar": "ORA_MCP_OFFICIAL_ORA_SPACE_TAVILY_SEARCH_APIKEY_0",
                            "prefix": "Bearer ",
                            "suffix": ""
                        }]
                    }]
                })
            )]
        );
        assert_eq!(rendered, RenderedMcpFile { bytes, digest });
    }

    /// The host never trusts the plugin's digest: a mismatch between the returned bytes and digest
    /// is rejected so a malformed render cannot write a file whose marker would claim another.
    #[tokio::test]
    async fn rejects_a_render_whose_digest_does_not_match_its_bytes() {
        let runtime = FakeRuntime {
            calls: Arc::new(Mutex::new(Vec::new())),
            wait_result: json!({}),
            render_result: json!({
                "bytes": "not-the-digested-bytes",
                "digest": "sha256:0000000000000000000000000000000000000000000000000000000000000000"
            }),
        };
        let desired = tavily_desired();

        assert_eq!(
            render_mcp_complete_file_with(&runtime, std::slice::from_ref(&desired)).await,
            Err(AgentEffectError::Ipc(
                "rendered file digest does not match its bytes".to_string()
            ))
        );
    }
}
