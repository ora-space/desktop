//! MCP settings cross the start frame while remaining bounded and absent from diagnostics.
use super::{agent_session, support::*};
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

/// Builds a start envelope independently from the Rust binding representation.
fn input(mcp: Value) -> ControllerToNodeMessage {
    let mut wire = agent_session::start().wire;
    wire["payload"]["spec"]["mcp"] = mcp;
    serde_json::from_value(wire).unwrap()
}

/// Includes each supported scalar so wire clients cannot drift to a string-only interpretation.
fn binding(id: &str) -> Value {
    json!({"plugin_id": id, "plugin_version": "1.2.0", "settings": {
        "token": "m938-control-secret", "count": 2.5, "enabled": true
    }})
}

/// Fragmented framing preserves explicit settings; old input and empty selections remain equal.
#[tokio::test]
async fn session_mcp_round_trip_and_old_spec() -> Result<(), TestError> {
    let message = input(json!([binding("official/acme.mcp")]));
    assert_eq!(round_trip_controller(message.clone()).await?, message);
    let old: ControllerToNodeMessage = serde_json::from_value(agent_session::start().wire)?;
    assert_eq!(input(json!([])), old);
    assert_eq!(round_trip_controller(old.clone()).await?, old);
    assert_eq!(
        serde_json::to_value(old)?["payload"]["spec"].get("mcp"),
        None
    );
    Ok(())
}

/// Every owning Debug boundary, including the whole command, must redact runtime values.
#[test]
fn session_mcp_debug_redacts_settings() {
    let message = input(json!([binding("official/acme.mcp")]));
    assert!(!format!("{message:?}").contains("m938-control-secret"));
    let ControllerToNodeMessage::StartAgentSession(message) = message else {
        unreachable!()
    };
    assert!(!format!("{:?}", message.payload.spec.mcp[0]).contains("m938-control-secret"));
}

/// Rejects invalid input on encoding as well as at the frame receive boundary.
#[test]
fn session_mcp_rejects_ambiguous_or_unbounded_selection() {
    let mut oversized = binding("official/acme.mcp");
    oversized["settings"] = json!({"token": "x".repeat(MAX_SESSION_MCP_SETTINGS_BYTES)});
    let mut wrong_version = binding("official/acme.mcp");
    wrong_version["plugin_version"] = json!("v1.2.0");
    let mut escaped = binding("official/acme.mcp");
    escaped["settings"] = json!({
        "k".repeat(64): "\n".repeat(MAX_SESSION_MCP_SETTINGS_BYTES - 128)
    });
    let mut key_heavy = binding("official/acme.mcp");
    key_heavy["settings"] = json!({
        "k".repeat(MAX_SESSION_MCP_SETTINGS_BYTES): "x"
    });
    for selection in [
        json!([binding("official/acme.mcp"), binding("official/acme.mcp")]),
        json!([binding("official/z.mcp"), binding("official/a.mcp")]),
        json!([binding("Official/acme.mcp")]),
        json!([wrong_version]),
        json!([oversized]),
        json!([escaped]),
        json!([key_heavy]),
        Value::Array(
            (0..=MAX_SESSION_MCP_SERVERS)
                .map(|i| binding(&format!("official/acme.mcp-{i:02}")))
                .collect(),
        ),
    ] {
        assert!(matches!(
            encode_controller_frame(&input(selection)),
            Err(FrameError::InvalidMessage(
                MessageValidationError::InvalidSessionMcp
            ))
        ));
    }
    let mut left = binding("official/a.mcp");
    let mut right = binding("official/b.mcp");
    left["settings"] = json!({"token": "x".repeat(40 * 1024)});
    right["settings"] = left["settings"].clone();
    assert!(encode_controller_frame(&input(json!([left, right]))).is_err());
    let mut exact = binding("official/acme.mcp");
    exact["settings"] = json!({"token":"x".repeat(MAX_SESSION_MCP_SETTINGS_BYTES - 12)});
    assert!(encode_controller_frame(&input(json!([exact]))).is_ok());
    let accepted = input(Value::Array(
        (0..MAX_SESSION_MCP_SERVERS)
            .map(|i| binding(&format!("official/acme.mcp-{i:02}")))
            .collect(),
    ));
    assert!(encode_controller_frame(&accepted).is_ok());
}
