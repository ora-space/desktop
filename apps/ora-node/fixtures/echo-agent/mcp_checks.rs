//! ACP setup checks: whether `mcpServers` carries exactly the plugin and host servers a test
//! configured. Only verdicts leave memory, never the secrets the tests pass in.

use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::Write;

/// Host MCP secret a test sends; it appears only here and in the test's in-memory input.
const HOST_MCP_SECRET: &str = "m1264-host-secret";

/// Records, per setup request, whether the host part of `mcpServers` is exactly the one test input
/// and follows every plugin server. Only the verdict reaches the file, never a host value.
pub(crate) fn record_host_mcp(method: &str, params: &Value) {
    let Ok(port) = std::fs::read_to_string("host-mcp-port") else {
        return;
    };
    let servers = params["mcpServers"].as_array().cloned().unwrap_or_default();
    let split = servers
        .iter()
        .position(|server| {
            server["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("ora-"))
        })
        .unwrap_or(servers.len());
    let (plugins, hosts) = servers.split_at(split);
    let header = |value: String| {
        vec![agent_client_protocol_schema::v1::HttpHeader::new(
            "authorization",
            value,
        )]
    };
    let expected = [
        agent_client_protocol_schema::v1::McpServerHttp::new(
            "ora-alpha",
            format!("http://127.0.0.1:{}/mcp/{HOST_MCP_SECRET}", port.trim()),
        )
        .headers(header(format!("Bearer {HOST_MCP_SECRET}"))),
        agent_client_protocol_schema::v1::McpServerHttp::new("ora-beta", "http://[::1]:41235/mcp"),
    ]
    .into_iter()
    .map(|server| {
        serde_json::to_value(agent_client_protocol_schema::v1::McpServer::Http(server))
            .unwrap_or(Value::Null)
    })
    .collect::<Vec<_>>();
    let verdict = if hosts.is_empty() {
        "none"
    } else if hosts == expected.as_slice()
        && plugins.iter().all(|server| {
            !server["name"]
                .as_str()
                .unwrap_or_default()
                .starts_with("ora-")
        })
    {
        "verified"
    } else {
        "mismatch"
    };
    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open("echo-host-mcp")
    {
        let _ = writeln!(file, "{method} plugins={} host={verdict}", plugins.len());
    }
}

/// Asserts the actual ACP frame entirely in memory; fixture secrets never enter transcript/files.
pub(crate) fn verify_mcp_frame(params: &Value) -> bool {
    let Some(command) = params
        .pointer("/mcpServers/0/command")
        .and_then(Value::as_str)
    else {
        return false;
    };
    // Host servers (`ora-` names) follow the plugin servers and are checked by `record_host_mcp`.
    let plugins = params["mcpServers"]
        .as_array()
        .map(|servers| {
            servers
                .iter()
                .filter(|server| {
                    !server["name"]
                        .as_str()
                        .is_some_and(|name| name.starts_with("ora-"))
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    std::path::Path::new(command).is_file()
        && command_is_in_required_view(std::path::Path::new(command))
        && Value::Array(plugins)
            == json!([
                {"name":"official/acme.stdio", "command":command,
                 "args":[params["cwd"], "arg-m938-runtime-secret"],
                 "env":[{"name":"TOKEN", "value":"m938-runtime-secret"}]},
                {"type":"http", "name":"official/acme.web", "url":"https://mcp.example.test/mcp",
                 "headers":[{"name":"Authorization", "value":"Bearer m938-runtime-secret"}]}
            ])
}

/// Separate workloads must receive the command through their view, never the private Node home.
fn command_is_in_required_view(command: &std::path::Path) -> bool {
    let Ok(bytes) = std::fs::read("mcp-command-view.json") else {
        return true;
    };
    let Ok(bounds) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    let (Some(workload), Some(home)) = (bounds["workload"].as_str(), bounds["home"].as_str())
    else {
        return false;
    };
    command.starts_with(workload) && !command.starts_with(home)
}
