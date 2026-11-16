//! Independent JSON fixtures for the lifecycle contract before transport admission is enabled.
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use serde_json::{Value, json};

const SECRET: &str = "lifecycle-fixture-secret";

/// A same-revision value conflict has its own code without changing any state snapshot.
#[test]
fn conflict_code_is_reply_local() {
    let wire = json!({"node":{"node_id":"node","incarnation_id":"instance"},
        "request_id":"request","result":"conflict","code":"configuration_revision_conflict"});
    let reply: SessionMcpReply = serde_json::from_value(wire.clone()).unwrap();
    reply.validate().unwrap();
    assert_eq!(serde_json::to_value(reply).unwrap(), wire);
    let mut invalid = wire;
    invalid["result"] = json!("accepted");
    assert!(
        serde_json::from_value::<SessionMcpReply>(invalid)
            .unwrap()
            .validate()
            .is_err()
    );
}

/// Health codes describe the probe, not configuration or ACP acceptance.
#[test]
fn health_and_convergence_have_distinct_failure_vocabularies() {
    assert!(
        serde_json::from_value::<SessionMcpHealth>(json!({
            "status":"unhealthy","code":"mcp_refresh_failed"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<SessionMcpFailure>(json!({
            "code":"mcp_probe_timeout"
        }))
        .is_err()
    );
    let health: SessionMcpHealth = serde_json::from_value(json!({
        "status":"unhealthy","code":"mcp_probe_timeout"
    }))
    .unwrap();
    assert_eq!(
        health,
        SessionMcpHealth::Unhealthy {
            code: SessionMcpProbeErrorCode::McpProbeTimeout
        }
    );
}

/// Uses JSON, not a Rust struct, to exercise a Controller's independently encoded request.
fn update() -> Value {
    json!({"protocol_version":1,"operation_id":"op","execution_id":"exec","payload":{
        "node_id":"node","request_id":"request","generation":1,"members":[{
            "state":"configured","plugin_id":"official/test.mcp","plugin_version":"1.2.0",
            "configuration_revision":0,"settings":{"token":SECRET,"number":1.5,"flag":true}
        }]
    }})
}

/// Settings survive the private input boundary but neither Debug nor identity can expose them.
#[test]
fn update_round_trips_and_identity_redacts_values() {
    let wire = update();
    let request: UpdateSessionMcpMessage = serde_json::from_value(wire.clone()).unwrap();
    request.validate().unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), wire);
    assert!(!format!("{request:?}").contains(SECRET));
    assert_eq!(
        serde_json::to_value(request.payload.members[0].identity()).unwrap(),
        json!({
            "state":"configured","plugin_id":"official/test.mcp","plugin_version":"1.2.0",
            "configuration_revision":0
        })
    );
}

/// Unavailable preserves selection membership, and cannot smuggle a values map or local path.
#[test]
fn unavailable_is_closed_and_value_free() {
    for code in ["mcp_plugin_missing", "mcp_setting_missing"] {
        let mut wire = update();
        wire["payload"]["members"][0] = json!({
            "state":"unavailable","plugin_id":"official/test.mcp","code":code
        });
        let request: UpdateSessionMcpMessage = serde_json::from_value(wire.clone()).unwrap();
        request.validate().unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), wire);
        for extra in ["settings", "path", "plugin_version"] {
            let mut invalid = wire.clone();
            invalid["payload"]["members"][0][extra] = json!(SECRET);
            assert!(serde_json::from_value::<UpdateSessionMcpMessage>(invalid).is_err());
        }
    }
}

/// JSON-safe counters and compact escaped Settings size are enforced on every whole input.
#[test]
fn input_bounds_are_checked_at_the_declared_edges() {
    let mut exact = update();
    exact["payload"]["generation"] = json!(MAX_SESSION_MCP_COUNTER);
    exact["payload"]["members"][0]["configuration_revision"] = json!(MAX_SESSION_MCP_COUNTER);
    exact["payload"]["members"][0]["settings"] = json!({
        "token":"x".repeat(MAX_SESSION_MCP_SETTINGS_BYTES - 12)
    });
    serde_json::from_value::<UpdateSessionMcpMessage>(exact.clone())
        .unwrap()
        .validate()
        .unwrap();
    let mut invalid = Vec::new();
    for generation in [0, MAX_SESSION_MCP_COUNTER + 1] {
        let mut wire = update();
        wire["payload"]["generation"] = json!(generation);
        invalid.push(wire);
    }
    let mut wire = update();
    wire["payload"]["members"][0]["configuration_revision"] = json!(MAX_SESSION_MCP_COUNTER + 1);
    invalid.push(wire);
    for (key, value) in [
        ("plugin_id", json!("Official/test.mcp")),
        ("plugin_version", json!("v1.2.0")),
        (
            "settings",
            json!({"token":"x".repeat(MAX_SESSION_MCP_SETTINGS_BYTES - 11)}),
        ),
        (
            "settings",
            json!({"token":"\n".repeat(MAX_SESSION_MCP_SETTINGS_BYTES / 2)}),
        ),
    ] {
        let mut wire = update();
        wire["payload"]["members"][0][key] = value;
        invalid.push(wire);
    }
    let mut wire = update();
    wire["payload"]["request_id"] = json!("r".repeat(129));
    invalid.push(wire);
    for wire in invalid {
        assert_eq!(
            serde_json::from_value::<UpdateSessionMcpMessage>(wire)
                .unwrap()
                .validate(),
            Err(MessageValidationError::InvalidSessionMcp)
        );
    }
}

/// Full-list validation never silently reorders, deduplicates, or ignores unavailable entries.
#[test]
fn input_requires_a_bounded_canonical_list() {
    for ids in [
        vec!["official/z", "official/a"],
        vec!["official/a", "official/a"],
        vec!["official/a"; MAX_SESSION_MCP_SERVERS + 1],
    ] {
        let mut wire = update();
        wire["payload"]["members"] = json!(
            ids.into_iter()
                .map(|id| json!({
                    "state":"unavailable","plugin_id":id,"code":"mcp_plugin_missing"
                }))
                .collect::<Vec<_>>()
        );
        assert!(
            serde_json::from_value::<UpdateSessionMcpMessage>(wire)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut wire = update();
    wire["payload"]["members"] = json!([]);
    serde_json::from_value::<UpdateSessionMcpMessage>(wire)
        .unwrap()
        .validate()
        .unwrap();
}

/// Manual probes allow startup generation zero but never the automatic attempt identifier zero.
#[test]
fn manual_probe_uses_exact_current_counters_and_canonical_id() {
    let wire = json!({"node_id":"node","request_id":"request","generation":0,
        "plugin_id":"official/test.mcp","probe_attempt":1});
    serde_json::from_value::<ProbeSessionMcp>(wire.clone())
        .unwrap()
        .validate()
        .unwrap();
    for (key, value) in [
        ("probe_attempt", json!(0)),
        ("probe_attempt", json!(MAX_SESSION_MCP_COUNTER + 1)),
        ("generation", json!(MAX_SESSION_MCP_COUNTER + 1)),
        ("plugin_id", json!("Official/test.mcp")),
    ] {
        let mut invalid = wire.clone();
        invalid[key] = value;
        assert!(
            serde_json::from_value::<ProbeSessionMcp>(invalid)
                .unwrap()
                .validate()
                .is_err()
        );
    }
}

/// The snapshot has no Settings-shaped fields; an ended/missing session cannot reveal a cache.
#[test]
fn snapshot_identity_and_terminal_reply_invariants() {
    let state = json!({"state_version":1,"desired":{"generation":0,"members":[]},
        "active":{"generation":0,"members":[]},"convergence":"current","members":[],
        "probe_capacity_available":true});
    serde_json::from_value::<SessionMcpState>(state.clone())
        .unwrap()
        .validate()
        .unwrap();
    for field in ["settings", "command", "env", "headers", "url"] {
        let mut invalid = state.clone();
        invalid[field] = json!(SECRET);
        assert!(serde_json::from_value::<SessionMcpState>(invalid).is_err());
    }
    for result in [
        "not_found",
        "session_ended",
        "not_enabled",
        "capacity_exceeded",
    ] {
        let mut reply = json!({"node":{"node_id":"node","incarnation_id":"instance"},
            "request_id":"request","result":result});
        serde_json::from_value::<SessionMcpReply>(reply.clone())
            .unwrap()
            .validate()
            .unwrap();
        reply["state"] = state.clone();
        assert!(
            serde_json::from_value::<SessionMcpReply>(reply)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut stale = state;
    stale["desired"]["generation"] = json!(1);
    assert!(
        serde_json::from_value::<SessionMcpState>(stale)
            .unwrap()
            .validate()
            .is_err()
    );
}
