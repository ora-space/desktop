//! Covers which method a discovery calls and what it makes of the answer, over a real plugin channel.

use super::{PipeProcess, PipeSpawner};
use crate::clock::SystemClock;
use crate::plugin_agent::{DiscoveryScope, PluginAgentError, discover};
use ora_plugin_protocol::{
    AGENT_DISCOVERY_BUDGET_MS, AgentCommand, AgentDiscoverResult, AgentModel, read_message,
    write_message,
};
use ora_plugin_runtime::{
    NoHostRequests, PluginLogSetup, PluginRuntime, PluginRuntimeConfig, PluginRuntimeError,
};
use pretty_assertions::assert_eq;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;
use tokio::sync::watch;

/// Runs one discovery against a plugin with these methods and answers.
///
/// Returns the outcome and the requests the plugin received. A null answer is none at all.
async fn discovery(
    methods: &[&str],
    answer: fn(&str) -> Value,
    scope: DiscoveryScope,
) -> (Result<AgentDiscoverResult, PluginAgentError>, Vec<Value>) {
    let directory = TempDir::new().unwrap();
    let entrypoint = directory.path().join("agent.ts");
    std::fs::write(&entrypoint, "").unwrap();
    let (stdin, mut reader) = tokio::io::duplex(/*max_buf_size*/ 65536);
    let (mut writer, stdout) = tokio::io::duplex(/*max_buf_size*/ 65536);
    let (exited, _) = watch::channel(false);
    let spawner = PipeSpawner(Mutex::new(Some(PipeProcess {
        stdin: Some(stdin),
        stdout: Some(stdout),
        exited: exited.clone(),
    })));
    let received = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&received);
    let registration = json!({"jsonrpc":"2.0", "method":"ora/register",
        "params":{"methods":methods, "emits":["agent/acp"]}});
    tokio::spawn(async move {
        write_message(&mut writer, &registration).await.unwrap();
        while let Ok(Some(request)) = read_message(&mut reader).await {
            let method = request["method"].as_str().unwrap();
            if method == "ora/shutdown" {
                break;
            }
            log.lock()
                .unwrap()
                .push(json!({"method":method, "params":request["params"]}));
            let mut reply = answer(method);
            if reply.is_null() {
                continue;
            }
            reply["jsonrpc"] = json!("2.0");
            reply["id"] = request["id"].clone();
            write_message(&mut writer, &reply).await.unwrap();
        }
        exited.send_replace(true);
    });
    let (_level, level) = watch::channel(ora_logging::LogLevel::Info);
    let (runtime, _notifications) = PluginRuntime::launch(
        &spawner,
        PluginRuntimeConfig {
            plugin_id: "official/test.agent".to_string(),
            deno_path: directory.path().join("unused-deno"),
            entrypoint,
            permissions: Vec::new(),
            cwd: None,
            environment: BTreeMap::new(),
            ready_timeout: Duration::from_secs(5),
            call_timeout: Duration::from_secs(5),
            shutdown_timeout: Duration::from_secs(1),
        },
        NoHostRequests,
        PluginLogSetup {
            root: directory.path().join("logs"),
            directory: directory.path().join("logs").join("agent"),
            host_session_id: "test".to_string(),
            generation: 1,
            level,
        },
    )
    .await
    .unwrap();
    let outcome = discover(&runtime, Path::new("project"), scope).await;
    runtime.shutdown_and_wait().await;
    let received = received.lock().unwrap().clone();
    (outcome, received)
}

/// Answers both methods, always with one command under the new name.
fn answer(method: &str) -> Value {
    let models = json!([{"id":"m1", "displayName":"Model 1", "default":true}]);
    match method {
        "agent/discover" => json!({"result":{"models":models,
            "commands":[{"name":"review", "description":"Review"}]}}),
        "agent/list_models" => json!({"result":{"models":models}}),
        other => panic!("unexpected request: {other}"),
    }
}

/// Builds what the answer above means, with these commands.
fn discovered(commands: Option<Vec<AgentCommand>>) -> AgentDiscoverResult {
    AgentDiscoverResult {
        models: vec![AgentModel {
            id: "m1".to_string(),
            display_name: "Model 1".to_string(),
            default: true,
        }],
        commands,
    }
}

/// Takes the deadline out of the only request, leaving what does not depend on the clock.
fn take_deadline(received: &mut [Value]) -> Option<i64> {
    let [request] = received else {
        panic!("expected one request: {received:?}");
    };
    request["params"]
        .as_object_mut()
        .unwrap()
        .remove("deadline")
        .map(|deadline| deadline.as_i64().unwrap())
}

/// A plugin that knows the new name is asked by it alone, and told whether commands are wanted.
#[tokio::test]
async fn the_new_method_is_called_alone_when_registered() {
    let review = AgentCommand {
        name: "review".to_string(),
        description: "Review".to_string(),
        input_hint: None,
    };
    for methods in [
        &["agent/discover"][..],
        &["agent/list_models", "agent/discover"][..],
    ] {
        for (scope, wanted, commands) in [
            (
                DiscoveryScope::ModelsAndCommands,
                true,
                Some(vec![review.clone()]),
            ),
            (DiscoveryScope::Models, false, None),
        ] {
            let (outcome, mut received) = discovery(methods, answer, scope).await;
            let deadline = take_deadline(&mut received);

            assert_eq!(outcome, Ok(discovered(commands)));
            assert_eq!(
                received,
                vec![json!({"method":"agent/discover",
                    "params":{"cwd":"project", "includeCommands":wanted}})]
            );
            assert!(deadline.is_some());
        }
    }
}

/// A plugin published before the new name still lists models, and its commands stay unknown.
#[tokio::test]
async fn an_older_plugin_is_asked_by_the_models_only_name() {
    let (outcome, received) = discovery(
        &["agent/list_models"],
        answer,
        DiscoveryScope::ModelsAndCommands,
    )
    .await;

    assert_eq!(outcome, Ok(discovered(None)));
    assert_eq!(
        received,
        vec![json!({"method":"agent/list_models", "params":{"cwd":"project"}})]
    );
}

/// A failed discovery is not retried under the old name, which would start the agent again.
#[tokio::test]
async fn a_failure_does_not_fall_back_to_the_old_method() {
    let (outcome, received) = discovery(
        &["agent/discover", "agent/list_models"],
        |_| json!({"error":{"code":-32000, "message":"no agent"}}),
        DiscoveryScope::ModelsAndCommands,
    )
    .await;

    assert!(outcome.is_err());
    assert_eq!(received.len(), 1);
}

/// The host tells the plugin the moment at which it stops waiting itself.
#[tokio::test(start_paused = true)]
async fn the_plugin_is_told_when_the_host_stops_waiting() {
    let budget = Duration::from_millis(AGENT_DISCOVERY_BUDGET_MS);
    for method in ["agent/discover", "agent/list_models"] {
        let asked = tokio::time::Instant::now();
        let before = SystemClock.now_timestamp_millis();
        let (outcome, mut received) = discovery(
            &[method],
            |_| Value::Null,
            DiscoveryScope::ModelsAndCommands,
        )
        .await;
        let after = SystemClock.now_timestamp_millis();
        let deadline = take_deadline(&mut received);

        assert_eq!(
            outcome,
            Err(PluginAgentError::from(PluginRuntimeError::CallTimeout))
        );
        assert_eq!(asked.elapsed().as_secs(), budget.as_secs());
        if method == "agent/list_models" {
            assert_eq!(deadline, None);
        } else {
            let budget = budget.as_millis() as i64;
            let deadline = deadline.unwrap();
            assert!((before + budget..=after + budget).contains(&deadline));
        }
    }
}
