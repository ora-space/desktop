//! Host MCP servers cross the start frame under one grammar the Controller mirrors through the
//! shared fixture, while their URL and headers stay out of every diagnostic.
use super::{agent_session, support::TestError, support::round_trip_controller};
use ora_node_protocol::*;
use pretty_assertions::assert_eq;
use serde::Deserialize;
use serde_json::{Value, json};

/// The fixture Controllers validate against too; its path is part of the published contract.
const FIXTURE: &str = include_str!("../../fixtures/host_mcp_servers.json");

/// One labelled URL, wrapped in a single server with no headers.
#[derive(Deserialize)]
struct UrlCase {
    url: String,
    accept: bool,
    note: String,
}

/// One labelled `host_mcp_servers` value.
#[derive(Deserialize)]
struct ListCase {
    host_mcp_servers: Value,
    accept: bool,
    note: String,
}

/// One labelled `host_mcp_servers` JSON text, for inputs a JSON value cannot express.
#[derive(Deserialize)]
struct RawCase {
    host_mcp_servers_json: String,
    accept: bool,
    note: String,
}

#[derive(Deserialize)]
struct Fixture {
    url_cases: Vec<UrlCase>,
    list_cases: Vec<ListCase>,
    raw_cases: Vec<RawCase>,
}

/// Parses the shared fixture.
fn fixture() -> Fixture {
    serde_json::from_str(FIXTURE).unwrap()
}

/// Builds a start envelope independently from the Rust binding representation.
fn wire(servers: Value) -> Value {
    let mut wire = agent_session::start().wire;
    wire["payload"]["spec"]["host_mcp_servers"] = servers;
    wire
}

/// Wraps one URL the way the fixture's `url_cases` define.
fn server_with_url(url: &str) -> Value {
    json!([{"name": "ora-test", "url": url, "headers": {}}])
}

/// Frames raw JSON text, bypassing the typed encoder so the receive boundary sees it verbatim.
fn received(payload: &[u8]) -> Result<ControllerToNodeMessage, FrameError> {
    let mut frame = vec![NODE_MESSAGE_FRAME_TYPE];
    frame.extend_from_slice(payload);
    decode_controller_frame(&frame)
}

/// Asserts one fixture verdict at the receive boundary and, when the value is typed, at encoding.
fn assert_verdict(payload: &[u8], accept: bool, note: &str) {
    match (received(payload), accept) {
        (Ok(message), true) => {
            encode_controller_frame(&message).unwrap_or_else(|error| {
                panic!("{note}: accepted on receive, refused on encode: {error:?}")
            });
        }
        (Err(FrameError::InvalidMessage(MessageValidationError::InvalidHostMcpServers)), false) => {
            // The typed value decoded, so the encoder must refuse the same input.
            let typed: ControllerToNodeMessage = serde_json::from_slice(payload).unwrap();
            assert!(
                matches!(
                    encode_controller_frame(&typed),
                    Err(FrameError::InvalidMessage(
                        MessageValidationError::InvalidHostMcpServers
                    ))
                ),
                "{note}: refused on receive, accepted on encode"
            );
        }
        // Shape errors (unknown or duplicate fields, missing headers) never form a typed value.
        (Err(FrameError::DecodeJson(_)), false) => {
            assert!(
                serde_json::from_slice::<ControllerToNodeMessage>(payload).is_err(),
                "{note}"
            );
        }
        (other, accept) => panic!("{note}: expected accept={accept}, got {other:?}"),
    }
}

/// Every fixture case reaches its labelled verdict on the Node.
#[test]
fn host_mcp_fixture_verdicts_hold_on_the_node() {
    let fixture = fixture();
    for case in &fixture.url_cases {
        let payload = serde_json::to_vec(&wire(server_with_url(&case.url))).unwrap();
        assert_verdict(
            &payload,
            case.accept,
            &format!("url {:?}: {}", case.url, case.note),
        );
    }
    for case in &fixture.list_cases {
        let payload = serde_json::to_vec(&wire(case.host_mcp_servers.clone())).unwrap();
        assert_verdict(&payload, case.accept, &case.note);
    }
    for case in &fixture.raw_cases {
        let placeholder = "\"__host_mcp_servers__\"";
        let text = serde_json::to_string(&wire(json!("__host_mcp_servers__")))
            .unwrap()
            .replace(placeholder, &case.host_mcp_servers_json);
        assert_verdict(text.as_bytes(), case.accept, &case.note);
    }
}

/// The fixture covers every rejection class and both sides of each closed bound.
#[test]
fn host_mcp_fixture_covers_each_bound_exactly() {
    let fixture = fixture();
    let accepted = |len: usize| {
        fixture
            .url_cases
            .iter()
            .find(|case| case.url.len() == len)
            .map(|case| case.accept)
    };
    assert_eq!(
        (
            accepted(MAX_HOST_MCP_URL_BYTES),
            accepted(MAX_HOST_MCP_URL_BYTES + 1)
        ),
        (Some(true), Some(false))
    );
    let lists = |servers: usize, headers: usize| {
        fixture
            .list_cases
            .iter()
            .filter(|case| {
                case.host_mcp_servers.as_array().is_some_and(|list| {
                    list.len() == servers
                        && list[0]["headers"]
                            .as_object()
                            .is_some_and(|h| h.len() == headers)
                })
            })
            .map(|case| case.accept)
            .collect::<Vec<_>>()
    };
    assert!(lists(MAX_HOST_MCP_SERVERS, 0).contains(&true));
    assert!(lists(MAX_HOST_MCP_SERVERS + 1, 0).contains(&false));
    assert!(lists(1, MAX_HOST_MCP_HEADERS).contains(&true));
    assert!(lists(1, MAX_HOST_MCP_HEADERS + 1).contains(&false));
    let total = |case: &ListCase| {
        case.host_mcp_servers.as_array().map_or(0, |list| {
            list.iter()
                .map(|server| {
                    server["url"].as_str().map_or(0, str::len)
                        + server["headers"].as_object().map_or(0, |headers| {
                            headers
                                .iter()
                                .map(|(name, value)| {
                                    name.len() + value.as_str().map_or(0, str::len)
                                })
                                .sum()
                        })
                })
                .sum::<usize>()
        })
    };
    let at = |bytes: usize| {
        fixture
            .list_cases
            .iter()
            .filter(|case| total(case) == bytes)
            .map(|case| case.accept)
            .collect::<Vec<_>>()
    };
    assert_eq!(at(MAX_HOST_MCP_TOTAL_BYTES), vec![true, true]);
    assert_eq!(at(MAX_HOST_MCP_TOTAL_BYTES + 1), vec![false]);
}

/// The grammar guarantees every accepted URL survives a WHATWG parse and serialization unchanged,
/// so the Agent's parser sees exactly what the Controller sent.
#[test]
fn host_mcp_accepted_urls_are_whatwg_fixed_points() {
    let fixture = fixture();
    for case in fixture.url_cases.iter().filter(|case| case.accept) {
        let parsed =
            url::Url::parse(&case.url).unwrap_or_else(|error| panic!("{}: {error}", case.url));
        assert_eq!(parsed.as_str(), case.url, "{}", case.note);
    }
}

/// Omitted and empty lists are the same input, the empty list is never written, and fragmented
/// framing preserves the servers in order with their headers.
#[tokio::test]
async fn host_mcp_round_trip_and_old_spec() -> Result<(), TestError> {
    let servers = json!([
        {"name": "ora-a", "url": "http://127.0.0.1:41234/mcp/3f1c", "headers": {"authorization": "Bearer m1264"}},
        {"name": "ora-b", "url": "http://[::1]:41235/mcp", "headers": {}}
    ]);
    let message: ControllerToNodeMessage = serde_json::from_value(wire(servers.clone()))?;
    assert_eq!(round_trip_controller(message.clone()).await?, message);
    assert_eq!(
        serde_json::to_value(&message)?["payload"]["spec"]["host_mcp_servers"],
        servers
    );
    let old: ControllerToNodeMessage = serde_json::from_value(agent_session::start().wire)?;
    let empty: ControllerToNodeMessage = serde_json::from_value(wire(json!([])))?;
    assert_eq!(empty, old);
    assert_eq!(
        serde_json::to_value(&empty)?["payload"]["spec"].get("host_mcp_servers"),
        None
    );
    Ok(())
}

/// URL and headers never reach `Debug`, and neither a validation nor a decode failure echoes them.
#[test]
fn host_mcp_values_never_reach_diagnostics() {
    let marker = "m1264-host-secret";
    let message: ControllerToNodeMessage = serde_json::from_value(wire(json!([{
        "name": "ora-test",
        "url": format!("http://127.0.0.1:41234/{marker}"),
        "headers": {"authorization": format!("Bearer {marker}")}
    }])))
    .unwrap();
    assert!(!format!("{message:?}").contains(marker));
    let invalid = serde_json::to_vec(&wire(json!([{
        "name": "ora-test",
        "url": format!("http://localhost:1/{marker}"),
        "headers": {"authorization": marker}
    }])))
    .unwrap();
    let error = received(&invalid).unwrap_err();
    assert!(!format!("{error} {error:?}").contains(marker));
    let duplicate = serde_json::to_string(&wire(json!("__x__")))
        .unwrap()
        .replace(
            "\"__x__\"",
            &format!(r#"[{{"name":"ora-test","url":"http://127.0.0.1:1/","headers":{{"x":"{marker}","x":"{marker}"}}}}]"#),
        );
    let error = received(duplicate.as_bytes()).unwrap_err();
    assert!(!format!("{error}").contains(marker));
}

/// Host servers are not plugin MCP: a session at the 16-binding cap still carries four of them.
#[test]
fn host_mcp_servers_do_not_count_against_the_plugin_cap() {
    let mut wire = wire(Value::Array(
        (0..MAX_HOST_MCP_SERVERS)
            .map(|i| json!({"name": format!("ora-host-{i}"), "url": "https://example.com/mcp", "headers": {}}))
            .collect(),
    ));
    wire["payload"]["spec"]["mcp"] = Value::Array(
        (0..MAX_SESSION_MCP_SERVERS)
            .map(|i| json!({"plugin_id": format!("official/acme.mcp-{i:02}"), "plugin_version": "1.2.0", "settings": {}}))
            .collect(),
    );
    let message: ControllerToNodeMessage = serde_json::from_value(wire).unwrap();
    let frame = encode_controller_frame(&message).unwrap();
    assert_eq!(decode_controller_frame(&frame).unwrap(), message);
}
