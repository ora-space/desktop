use ora_plugin_manager::{
    DirectoryRuntimeAssetSource, PluginManagerConfig, PluginRuntimeAssets, RuntimeAssetStore,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// Returns the repository root from the plugin-manager crate location.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| panic!("expected workspace root"))
        .to_path_buf()
}

/// Resolves the ignored, digest-verified runtime resources prepared by the explicit task.
pub fn prepared_runtime_resources() -> PathBuf {
    let resources = workspace_root().join("runtime-assets").join("prepared");
    assert!(resources.join("runtime-manifest.json").is_file());
    resources
}

/// Deploys runtime resources and injects only the fixed Windows base environment.
pub async fn prepare_runtime_assets(config: &PluginManagerConfig) -> PluginRuntimeAssets {
    let assets = RuntimeAssetStore::new(
        config.plugin_runtime_dir(),
        Arc::new(DirectoryRuntimeAssetSource::new(
            prepared_runtime_resources(),
        )),
    )
    .prepare()
    .await
    .unwrap_or_else(|error| panic!("expected runtime deploy: {error}"));
    let mut assets = PluginRuntimeAssets::from_runtime_lease(assets)
        .await
        .unwrap_or_else(|error| panic!("expected runtime assets: {error}"));
    for key in ["SystemRoot", "WINDIR", "TEMP", "TMP"] {
        let value = std::env::var_os(key)
            .unwrap_or_else(|| panic!("expected required Windows environment {key}"));
        assets = assets.with_environment(key, value);
    }
    assets
}

/// Packs an author-style public-SDK Agent source into an isolated materialized artifact.
pub fn pack_agent_fixture(test_root: &Path, plugin_id: &str) -> PathBuf {
    let source = test_root.join("source");
    let artifact = test_root.join("artifact");
    let entry = source.join("src").join("index.ts");
    std::fs::create_dir_all(
        source
            .join("node_modules")
            .join("@ora-space")
            .join("plugin-sdk")
            .join("dist")
            .join("agent"),
    )
    .unwrap_or_else(|error| panic!("expected fixture SDK directory: {error}"));
    std::fs::create_dir_all(
        entry
            .parent()
            .unwrap_or_else(|| panic!("expected entry parent")),
    )
    .unwrap_or_else(|error| panic!("expected fixture source directory: {error}"));

    let sdk_root = workspace_root().join("packages").join("plugin-sdk");
    std::fs::copy(
        sdk_root.join("package.json"),
        source
            .join("node_modules")
            .join("@ora-space")
            .join("plugin-sdk")
            .join("package.json"),
    )
    .unwrap_or_else(|error| panic!("expected public SDK package metadata: {error}"));
    std::fs::copy(
        sdk_root.join("dist").join("agent").join("index.js"),
        source
            .join("node_modules")
            .join("@ora-space")
            .join("plugin-sdk")
            .join("dist")
            .join("agent")
            .join("index.js"),
    )
    .unwrap_or_else(|error| {
        panic!("expected built public SDK; run its build before runtime E2E: {error}")
    });
    std::fs::write(&entry, agent_source())
        .unwrap_or_else(|error| panic!("expected Agent author source: {error}"));
    std::fs::write(
        source.join("package.json"),
        format!(
            r#"{{"name":"@ora/e2e","version":"0.1.0","type":"module","ora":{{"manifestVersion":1,"id":"{plugin_id}","displayName":"E2E","kind":"agent","main":"dist/index.js","engines":{{"ora":">=0.1.0 <0.2.0","pluginApi":1,"bun":">=1.0.0 <2.0.0"}},"contributes":{{"agents":[{{"id":"example","displayName":"Example","contractVersion":1}}]}}}}}}"#
        ),
    )
    .unwrap_or_else(|error| panic!("expected Agent package metadata: {error}"));

    let resources = prepared_runtime_resources();
    let bun = resources.join("bun.exe");
    let pack_cli = sdk_root.join("dist").join("pack").join("cli.js");
    let output = Command::new(&bun)
        .arg(format!(
            "--config={}",
            resources.join("empty-bunfig.toml").display()
        ))
        .arg("--no-env-file")
        .arg("--no-macros")
        .arg("run")
        .arg("--no-install")
        .arg(pack_cli)
        .arg("pack")
        .arg("--source")
        .arg(&source)
        .arg("--entry")
        .arg("src/index.ts")
        .arg("--output")
        .arg(&artifact)
        .arg("--bun")
        .arg(&bun)
        .current_dir(&source)
        .output()
        .unwrap_or_else(|error| panic!("expected pinned Bun pack command: {error}"));
    assert!(
        output.status.success(),
        "pinned pack failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(artifact.join("dist").join("index.js").is_file());
    artifact
}

/// Writes one valid catalog-only Workbench package beside an Agent fixture.
#[allow(dead_code)] // Integration test crates compile shared support independently.
pub fn write_workbench_fixture(root: &Path, plugin_id: &str) {
    std::fs::create_dir_all(root)
        .unwrap_or_else(|error| panic!("expected Workbench directory: {error}"));
    std::fs::write(
        root.join("package.json"),
        format!(
            r#"{{"name":"@ora/e2e-workbench","version":"0.1.0","type":"module","ora":{{"manifestVersion":1,"id":"{plugin_id}","displayName":"E2E Workbench","kind":"workbench","engines":{{"ora":">=0.1.0 <0.2.0"}},"contributes":{{"workbench":{{"schemaVersion":1}}}}}}}}"#
        ),
    )
    .unwrap_or_else(|error| panic!("expected Workbench manifest: {error}"));
}

/// Returns the author source used to exercise structural ABI, stream, and cancellation paths.
fn agent_source() -> &'static str {
    r#"
import { defineAgentPlugin } from "@ora-space/plugin-sdk/agent";

const provider = {
  id: "example",
  contractVersion: 1,
  async discoverInstallations() {
    return {
      installations: [],
      diagnostics: [{ kind: "notFound", message: "No installations found" }],
    };
  },
  async getConfigurationSummary() { return { items: [] }; },
  async listSkills() { return { items: [] }; },
  async listMcpServers() { return { items: [] }; },
  async listConversations() { return { items: [] }; },
  async *startConversation() {
    yield { kind: "conversationStarted", conversationId: "conversation" };
    yield { kind: "textDelta", channel: "assistant", text: "hello" };
    return { conversationId: "conversation", turnId: "turn", finishReason: "completed" };
  },
  async *sendMessage(call) {
    yield { kind: "textDelta", channel: "assistant", text: "pending" };
    await new Promise((resolve, reject) => {
      if (call.signal.aborted) {
        reject(new Error("cancelled"));
        return;
      }
      call.signal.addEventListener("abort", () => reject(new Error("cancelled")), { once: true });
    });
    return { conversationId: "conversation", turnId: "turn", finishReason: "completed" };
  },
  async cancelConversation() { return { disposition: "accepted" }; }
};

export default defineAgentPlugin({
  kind: "agent",
  pluginApi: 1,
  async activate() { return { providers: [provider] }; }
});
"#
}
