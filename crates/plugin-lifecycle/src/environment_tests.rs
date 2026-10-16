//! Real Deno processes verify both environment delivery paths without printing credential values.

use crate::{
    ChildProcessEnvironmentProvider, DenoPermission, DenoPluginRuntimeLauncher,
    PluginLaunchRequest, PluginProcessHost, PluginRuntime, PluginRuntimeLauncher,
    PluginRuntimeTimeouts,
};
use ora_plugin_runtime::HostRequestHandler;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone)]
struct Snapshot;

impl ChildProcessEnvironmentProvider for Snapshot {
    /// Returns the same fixture snapshot regardless of workspace.
    fn environment(&self, _id: &str, _cwd: &Path) -> Result<BTreeMap<String, String>, String> {
        Ok(self.plugin_environment("fixture"))
    }
    /// A synthetic value only; scripts record a boolean receipt rather than this value.
    fn plugin_environment(&self, _id: &str) -> BTreeMap<String, String> {
        BTreeMap::from([("ORA_SKILL_FIXTURE".into(), "test-only-value".into())])
    }
}

/// Agent launcher inheritance reaches directly spawned children; non-Agent launchers receive none.
#[tokio::test]
async fn real_agent_and_direct_child_share_environment_without_non_agent_injection() {
    ora_logging::initialize_test_clock();
    let root = tempfile::tempdir().unwrap();
    for agent in [true, false] {
        let receipt = root.path().join(format!("receipt-{agent}.json"));
        let script = root.path().join("main.js");
        let source = format!(
            r#"
const present = Deno.env.get("ORA_SKILL_FIXTURE") === "test-only-value";
const child = await new Deno.Command(Deno.execPath(), {{args:["eval", 'Deno.exit(Deno.env.get("ORA_SKILL_FIXTURE") === "test-only-value" ? 0 : 1)'], stdout:"null", stderr:"null"}}).output();
Deno.writeTextFileSync({}, JSON.stringify([present, child.success]));
const payload = new TextEncoder().encode(JSON.stringify({{jsonrpc:"2.0",method:"ora/register",params:{{methods:[],emits:[]}}}}));
const frame = new Uint8Array(payload.length + 5);
new DataView(frame.buffer).setUint32(0, payload.length + 1); frame[4] = 1; frame.set(payload,5);
Deno.stdout.writeSync(frame);
setInterval(() => {{}}, 1000);
"#,
            serde_json::to_string(&receipt).unwrap()
        );
        std::fs::write(&script, source).unwrap();
        let launcher = DenoPluginRuntimeLauncher::with_environment_provider(
            PluginRuntimeTimeouts::default(),
            Snapshot,
        );
        let launched = launcher
            .launch(
                PluginLaunchRequest {
                    plugin_id: ora_domain::PluginId::parse("official/fixture").unwrap(),
                    deno_path: PathBuf::from("deno"),
                    entrypoint: script,
                    package_root: root.path().to_path_buf(),
                    data_dir: root.path().to_path_buf(),
                    // Instrumented non-Agent fixture can inspect its own environment, but must receive no injection.
                    permissions: vec![
                        DenoPermission::AllowRead(crate::ReadScope::Everything),
                        DenoPermission::AllowEnv,
                        DenoPermission::AllowRun,
                        DenoPermission::AllowWrite(root.path().to_path_buf()),
                    ],
                    allow_childprocess: agent,
                },
                ora_plugin_runtime::PluginLogSetup {
                    root: root.path().join("logs"),
                    directory: root.path().join("logs/official/fixture"),
                    host_session_id: "test".into(),
                    generation: if agent { 1 } else { 2 },
                    level: tokio::sync::watch::channel(ora_logging::LogLevel::Info).1,
                },
            )
            .await
            .unwrap();
        let result: Vec<bool> =
            serde_json::from_str(&std::fs::read_to_string(receipt).unwrap()).unwrap();
        pretty_assertions::assert_eq!(result, vec![agent, agent]);
        launched.runtime.stop().await.unwrap();
    }
}

/// Host spawn supplies the same snapshot both with an explicit workspace and without cwd.
#[tokio::test]
async fn real_host_child_receives_snapshot_with_or_without_cwd() {
    let root = tempfile::tempdir().unwrap();
    let host = PluginProcessHost::with_environment_provider(
        "official/fixture",
        root.path().to_path_buf(),
        ora_process::TokioProcessSpawner::new(),
        Snapshot,
    );
    for with_cwd in [true, false] {
        let receipt = root.path().join(format!("host-{with_cwd}.txt"));
        let script = format!(
            "Deno.writeTextFileSync({}, Deno.env.get('ORA_SKILL_FIXTURE') === 'test-only-value' ? 'ok' : 'missing')",
            serde_json::to_string(&receipt).unwrap()
        );
        let mut request = serde_json::json!({"command":"deno","args":["eval",script]});
        if with_cwd {
            request["cwd"] = serde_json::json!(root.path());
        }
        host.handle(crate::CHILDPROCESS_SPAWN_METHOD, request)
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(15), async {
            while !receipt.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        pretty_assertions::assert_eq!(std::fs::read_to_string(receipt).unwrap(), "ok");
    }
    host.kill_all().await;
}
