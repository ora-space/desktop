use std::collections::HashSet;

use ora_domain::PluginId;
use ora_plugin_lifecycle::{InboundNotification, PluginGenerationKey};
use ora_plugin_runtime::{
    PluginEffectCoordination, PluginEffectResource, PluginRegistration, PluginRuntimeError,
};
use pretty_assertions::assert_eq;
use serde_json::json;
use tokio::sync::mpsc;

use super::control::{PluginAgentError, verify_agent_contract};
use super::effect::{AgentEffectError, registered_consumer_declaration};
use super::inbound::{discard_frames_before_start, spawn_frame_forwarding};

/// The exact materialization format an agent plugin must declare, as `plugin-sdk` publishes it.
///
/// Spelled out here rather than read from `ora-effect` so the two are compared rather than
/// derived from one another: this is the string that travels over the wire from a plugin built
/// against the SDK, and a rename on either side has to fail here instead of silently agreeing.
const SKILL_DIRECTORY_FORMAT: &str = "ora/skill-directory.v1";

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
            "agent/list_models".to_string(),
        ]),
        emits: HashSet::from(["agent/acp".to_string()]),
        effect_resources: Vec::new(),
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
    registration.methods.remove("agent/list_models");

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
        Some(vec!["agent/list_models", "agent/stop"])
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
    registration.effect_resources = vec![PluginEffectResource {
        workspace_relative_path: ".codex/skills".to_string(),
        materialization_format: SKILL_DIRECTORY_FORMAT.to_string(),
        coordination: PluginEffectCoordination::QuiesceBeforeMutation,
    }];

    assert_eq!(
        verify_agent_contract(&registration),
        Err(PluginAgentError::ContractIncomplete(
            "missing Effect methods effect/coordinate, effect/reactivate, effect/verify_ready"
                .to_string()
        ))
    );
}

/// The two reserved startup codes are the only ones classified apart, and the message a plugin
/// chose survives on the terminal one — nothing else ever reports why that package cannot run.
#[test]
fn classifies_the_two_reserved_agent_startup_codes_apart_from_every_other_failure() {
    let classify = |code: i64| {
        PluginAgentError::from(PluginRuntimeError::Remote {
            code,
            message: "opencode is unusable".to_string(),
        })
    };

    assert_eq!(
        [classify(-32001), classify(-32002), classify(-32000)],
        [
            PluginAgentError::AgentNotInstalled,
            PluginAgentError::AgentUnusable("opencode is unusable".to_string()),
            PluginAgentError::Failed(
                "plugin method failed with code -32000: opencode is unusable".to_string()
            ),
        ]
    );
}

/// Builds one plugin registration declaring a single Skill Resource in the given format.
fn registration_declaring(format: &str) -> PluginRegistration {
    let mut registration = complete_registration();
    registration.methods.insert("effect/coordinate".to_string());
    registration.methods.insert("effect/reactivate".to_string());
    registration
        .methods
        .insert("effect/verify_ready".to_string());
    registration.effect_resources = vec![PluginEffectResource {
        workspace_relative_path: ".opencode/skills".to_string(),
        materialization_format: format.to_string(),
        coordination: PluginEffectCoordination::QuiesceBeforeMutation,
    }];
    registration
}

/// The format `plugin-sdk` tells plugins to declare is the one this host accepts.
///
/// Pinned because the two sides agree only by string: a plugin declaring anything else is not
/// degraded but abandoned for the rest of the process, so a drift between the SDK's published
/// constant and `MaterializationFormat::skill_directory_v1` breaks every agent that ships Skills.
#[test]
fn accepts_the_skill_format_the_sdk_publishes() {
    let plugin_id = PluginId::new("official", "example.agent").expect("plugin id");

    let declaration = registered_consumer_declaration(
        &plugin_id,
        &registration_declaring(SKILL_DIRECTORY_FORMAT),
    )
    .expect("the documented format is accepted");

    let resources = declaration
        .expect("a declared Resource produces a Consumer")
        .resources;
    assert_eq!(
        resources
            .iter()
            .map(|resource| resource.materialization_format.as_str())
            .collect::<Vec<_>>(),
        vec![SKILL_DIRECTORY_FORMAT]
    );
}

/// Any unknown format is refused instead of being mapped onto a supported Effect format.
#[test]
fn rejects_a_materialization_format_this_host_cannot_serve() {
    let plugin_id = PluginId::new("official", "example.agent").expect("plugin id");

    let error =
        registered_consumer_declaration(&plugin_id, &registration_declaring("skill_directory.v1"))
            .expect_err("an unknown format is refused");

    assert_eq!(
        error,
        AgentEffectError::InvalidDeclaration(
            "unsupported Effect materialization format skill_directory.v1".to_string()
        )
    );
}

/// A plugin declaring no Resource is not a Consumer at all, rather than an empty one.
#[test]
fn declares_no_consumer_for_a_plugin_that_owns_nothing_on_disk() {
    let plugin_id = PluginId::new("official", "example.agent").expect("plugin id");

    assert_eq!(
        registered_consumer_declaration(&plugin_id, &complete_registration()),
        Ok(None)
    );
}

/// Unpublished MCP file-materialization formats are ignored so Skill Effect still registers.
#[test]
fn skips_unpublished_mcp_file_materialization_resources() {
    let plugin_id = PluginId::new("official", "example.agent").expect("plugin id");
    let mut registration = complete_registration();
    registration.effect_resources = vec![
        PluginEffectResource {
            workspace_relative_path: ".opencode/opencode.json".to_string(),
            materialization_format: "ora/opencode-mcp-config.v1".to_string(),
            coordination: PluginEffectCoordination::QuiesceBeforeMutation,
        },
        PluginEffectResource {
            workspace_relative_path: ".claude/.mcp.json".to_string(),
            materialization_format: "ora/claude-mcp-config.v1".to_string(),
            coordination: PluginEffectCoordination::QuiesceBeforeMutation,
        },
        PluginEffectResource {
            workspace_relative_path: ".opencode/skills".to_string(),
            materialization_format: SKILL_DIRECTORY_FORMAT.to_string(),
            coordination: PluginEffectCoordination::QuiesceBeforeMutation,
        },
    ];

    let declaration = registered_consumer_declaration(&plugin_id, &registration)
        .expect("MCP formats are skipped")
        .expect("Skill Resource remains");
    assert_eq!(
        declaration
            .resources
            .iter()
            .map(|resource| resource.materialization_format.as_str())
            .collect::<Vec<_>>(),
        vec![SKILL_DIRECTORY_FORMAT]
    );
}

/// An agent that only declared the unpublished MCP Resource is not an Effect Consumer.
#[test]
fn treats_mcp_only_materialization_as_no_consumer() {
    let plugin_id = PluginId::new("official", "example.agent").expect("plugin id");
    let mut registration = complete_registration();
    registration.effect_resources = vec![PluginEffectResource {
        workspace_relative_path: ".opencode/opencode.json".to_string(),
        materialization_format: "ora/opencode-mcp-config.v1".to_string(),
        coordination: PluginEffectCoordination::QuiesceBeforeMutation,
    }];

    assert_eq!(
        registered_consumer_declaration(&plugin_id, &registration),
        Ok(None)
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
