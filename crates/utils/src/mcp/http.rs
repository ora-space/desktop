//! Streamable HTTP MCP probe: initialize + tools/list, then optional session DELETE.

use super::protocol::{
    JsonRpcResponse, ProbeError, accept_result, initialize_request, initialized_notification,
    tools_list_request,
};
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Method, StatusCode};
use serde_json::Value;
use std::time::Duration;
use url::Url;

/// Streamable HTTP session header assigned by the server during `initialize`.
const SESSION_HEADER: &str = "mcp-session-id";

/// POSTs initialize and tools/list to a Streamable HTTP endpoint within one hard deadline.
///
/// The overall timeout bounds the whole handshake — including connect, TLS, every POST, and the
/// teardown DELETE — so a slow peer can never stretch a probe into several per-request timeouts.
pub async fn probe_http(
    url: Url,
    headers: Vec<(String, String)>,
    overall_timeout: Duration,
) -> Result<(), ProbeError> {
    match tokio::time::timeout(overall_timeout, run_http(url, headers, overall_timeout)).await {
        Ok(result) => result,
        Err(_) => Err(ProbeError::Timeout),
    }
}

async fn run_http(
    url: Url,
    headers: Vec<(String, String)>,
    overall_timeout: Duration,
) -> Result<(), ProbeError> {
    let client = Client::builder()
        // Per-request guard only; the caller's outer timeout stays the hard bound.
        .timeout(overall_timeout)
        .connect_timeout(overall_timeout.min(Duration::from_secs(5)))
        .build()
        .map_err(|_| ProbeError::HttpUnreachable)?;

    let user_headers = build_user_headers(&headers)?;
    let (initialize_body, session_id) = post_rpc(
        &client,
        &url,
        &user_headers,
        None,
        &initialize_request(1),
        ExpectBody::Yes,
    )
    .await?;
    let response = parse_rpc_payload(&initialize_body)?;
    if !accept_result(&response, 1) {
        return Err(ProbeError::HandshakeFailed);
    }

    post_rpc(
        &client,
        &url,
        &user_headers,
        session_id.as_deref(),
        &initialized_notification(),
        ExpectBody::No,
    )
    .await?;

    let (tools_body, _) = post_rpc(
        &client,
        &url,
        &user_headers,
        session_id.as_deref(),
        &tools_list_request(2),
        ExpectBody::Yes,
    )
    .await
    // Once `initialize` succeeded the handshake is proven; any tools/list failure — transport
    // level or protocol level — means the tools surface could not be listed.
    .map_err(|error| match error {
        ProbeError::HandshakeFailed => ProbeError::ToolsUnavailable,
        other => other,
    })?;
    let tools_response =
        parse_rpc_payload(&tools_body).map_err(|_| ProbeError::ToolsUnavailable)?;
    if !accept_result(&tools_response, 2) {
        return Err(ProbeError::ToolsUnavailable);
    }

    if let Some(session_id) = session_id {
        // Explicit teardown is best-effort: servers MAY reject it (405) or already be gone.
        let _ = delete_session(&client, &url, &user_headers, &session_id).await;
    }
    Ok(())
}

/// Whether one POST must return a JSON-RPC message body.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ExpectBody {
    Yes,
    No,
}

fn build_user_headers(headers: &[(String, String)]) -> Result<HeaderMap, ProbeError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|_| ProbeError::HandshakeFailed)?;
        let header_value = HeaderValue::from_str(value).map_err(|_| ProbeError::HandshakeFailed)?;
        map.append(header_name, header_value);
    }
    Ok(map)
}

/// Sends one JSON-RPC message and returns the raw response body plus the tracked session id.
///
/// Never surfaces response text: bodies may carry third-party error detail, and callers only
/// receive the stable classification.
async fn post_rpc(
    client: &Client,
    url: &Url,
    user_headers: &HeaderMap,
    session_id: Option<&str>,
    body: &Value,
    expect_body: ExpectBody,
) -> Result<(String, Option<String>), ProbeError> {
    let mut request = client
        .request(Method::POST, url.clone())
        .header(ACCEPT, "application/json, text/event-stream")
        .header(CONTENT_TYPE, "application/json")
        .headers(user_headers.clone())
        .json(body);
    if let Some(session_id) = session_id {
        request = request.header(SESSION_HEADER, session_id);
    }

    let response = request.send().await.map_err(map_transport_error)?;
    let status = response.status();
    let next_session = response
        .headers()
        .get(SESSION_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .or_else(|| session_id.map(str::to_owned));

    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(ProbeError::HttpUnauthorized);
    }
    if status.is_client_error() || status.is_server_error() {
        return Err(ProbeError::HttpServerError);
    }
    // 202 Accepted carries no message body; it is the expected ack for notifications.
    if expect_body == ExpectBody::No {
        return Ok((String::new(), next_session));
    }

    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let text = response
        .text()
        .await
        .map_err(|_| ProbeError::HttpUnreachable)?;
    if content_type.contains("text/event-stream") {
        let payload = first_sse_data(&text).ok_or(ProbeError::HandshakeFailed)?;
        Ok((payload, next_session))
    } else {
        Ok((text, next_session))
    }
}

async fn delete_session(
    client: &Client,
    url: &Url,
    user_headers: &HeaderMap,
    session_id: &str,
) -> Result<(), ProbeError> {
    client
        .request(Method::DELETE, url.clone())
        .headers(user_headers.clone())
        .header(SESSION_HEADER, session_id)
        .send()
        .await
        .map_err(map_transport_error)?;
    Ok(())
}

fn parse_rpc_payload(text: &str) -> Result<JsonRpcResponse, ProbeError> {
    serde_json::from_str(text.trim()).map_err(|_| ProbeError::HandshakeFailed)
}

/// Extracts the first SSE `data:` payload, which carries one JSON-RPC message for this probe.
fn first_sse_data(body: &str) -> Option<String> {
    let mut data_lines = Vec::new();
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            data_lines.push(rest.trim_start().to_owned());
        } else if line.is_empty() && !data_lines.is_empty() {
            break;
        }
    }
    if data_lines.is_empty() {
        None
    } else {
        Some(data_lines.join("\n"))
    }
}

/// Classifies reqwest failures without copying any OS or TLS error text into the result.
fn map_transport_error(error: reqwest::Error) -> ProbeError {
    if error.is_timeout() {
        ProbeError::Timeout
    } else {
        // Connect, DNS, TLS, redirect, and body-decode failures all mean the endpoint could not
        // be reached as a usable MCP peer.
        ProbeError::HttpUnreachable
    }
}
