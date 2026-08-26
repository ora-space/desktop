use std::collections::HashSet;

use ora_domain::PluginId;
use ora_plugin_lifecycle::{InboundNotification, PluginGenerationKey};
use ora_plugin_runtime::{PluginEffectCoordination, PluginEffectSurface, PluginRegistration};
use pretty_assertions::assert_eq;
use serde_json::json;
use tokio::sync::mpsc;

use super::control::{PluginAgentError, verify_agent_contract};
use super::inbound::{discard_frames_before_start, spawn_frame_forwarding};

/// Builds one notification as the lifecycle pump delivers it for the example agent plugin.
fn notification(method: &str, params: serde_json::Value) -> InboundNotification {
    InboundNotification {
        plugin_id: PluginId::new("official", "example.agent").expect("plugin id"),
        generation: PluginGenerationKey(1),
        method: method.to_string(),
        params,
    }
}

/// Builds a registration that satisfies the whole agent contract.
fn complete_registration() -> PluginRegistration {
    PluginRegistration {
        methods: HashSet::from([
            "agent/start".to_string(),
            "agent/stop".to_string(),
            "agent/listModels".to_string(),
        ]),
        emits: HashSet::from(["agent/acp".to_string()]),
        effect_surfaces: Vec::new(),
    }
}

/// A plugin that declares the whole contract is accepted without further checks.
#[test]
fn accepts_a_complete_agent_contract() {
    assert_eq!(verify_agent_contract(&complete_registration()), Ok(()));
}

/// Every missing control method is named at once so one restart surfaces the whole gap.
#[test]
fn rejects_a_registration_missing_control_methods() {
    let mut registration = complete_registration();
    registration.methods.remove("agent/stop");
    registration.methods.remove("agent/listModels");

    let error = verify_agent_contract(&registration).unwrap_err();

    let PluginAgentError::ContractIncomplete(detail) = error else {
        panic!("expected an incomplete contract");
    };
    assert_eq!(
        detail.strip_prefix("missing methods ").map(|methods| {
            let mut methods = methods.split(", ").collect::<Vec<_>>();
            methods.sort_unstable();
            methods
        }),
        Some(vec!["agent/listModels", "agent/stop"])
    );
}

/// A plugin that cannot emit ACP frames can never serve a session, so it fails at handshake.
#[test]
fn rejects_a_registration_that_cannot_emit_acp() {
    let mut registration = complete_registration();
    registration.emits.clear();

    assert_eq!(
        verify_agent_contract(&registration),
        Err(PluginAgentError::ContractIncomplete(
            "missing emitted method agent/acp".to_string()
        ))
    );
}

/// A coordinated surface is rejected unless the plugin can establish and release its barrier.
#[test]
fn rejects_a_surface_without_effect_control_methods() {
    let mut registration = complete_registration();
    registration.effect_surfaces = vec![PluginEffectSurface {
        workspace_relative_path: ".codex/skills".to_string(),
        materialization_format: "skill_directory.v1".to_string(),
        coordination: PluginEffectCoordination::WaitForIdleAndRestart,
    }];

    assert_eq!(
        verify_agent_contract(&registration),
        Err(PluginAgentError::ContractIncomplete(
            "missing Effect methods effect/waitForIdle, effect/restart".to_string()
        ))
    );
}

/// Frames that arrived before the agent started belong to no connection and are dropped.
#[tokio::test]
async fn discards_frames_that_arrived_before_the_agent_started() {
    let (sender, mut notifications) = mpsc::unbounded_channel();
    for index in 0..3 {
        sender
            .send(notification("agent/acp", json!({ "id": index })))
            .expect("queue early frame");
    }

    discard_frames_before_start(&mut notifications, "example.agent");
    sender
        .send(notification("agent/acp", json!({ "id": 99 })))
        .expect("queue live frame");
    let mut messages = spawn_frame_forwarding(notifications, "example.agent".to_string());

    assert_eq!(
        messages
            .recv()
            .await
            .expect("receive frame")
            .expect("frame is not a failure"),
        json!({ "id": 99 })
    );
}

/// Unusable single frames are dropped so one bad payload cannot end every live session.
#[tokio::test]
async fn drops_unusable_frames_without_failing_the_connection() {
    let (sender, notifications) = mpsc::unbounded_channel();
    for notification in [
        notification("agent/modelsChanged", json!({})),
        notification("agent/acp", json!("not an object")),
        notification(
            "agent/acp",
            json!({ "jsonrpc": "2.0", "method": "session/update" }),
        ),
    ] {
        sender.send(notification).expect("queue notification");
    }
    let mut messages = spawn_frame_forwarding(notifications, "example.agent".to_string());

    assert_eq!(
        messages
            .recv()
            .await
            .expect("receive frame")
            .expect("frame is not a failure"),
        json!({ "jsonrpc": "2.0", "method": "session/update" })
    );
}
