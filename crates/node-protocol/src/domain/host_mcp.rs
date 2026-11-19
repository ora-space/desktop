//! Controller-provided HTTP MCP servers attached to every ACP setup of one Agent session.
//!
//! Unlike plugin MCP these are not an installed package selection: the Controller that serves them
//! owns their authorization, and the Node only delivers them. Their URL and headers are runtime
//! values, so `Debug` redacts both and the Node persists nothing but the names.
mod url;

use crate::MessageValidationError;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Most host servers one session may carry.
pub const MAX_HOST_MCP_SERVERS: usize = 4;
/// Most request headers one host server may carry.
pub const MAX_HOST_MCP_HEADERS: usize = 8;
/// Largest URL of one host server, in bytes.
pub const MAX_HOST_MCP_URL_BYTES: usize = 2 * 1024;
/// Largest sum of every URL, header name and header value of one session, in bytes.
pub const MAX_HOST_MCP_TOTAL_BYTES: usize = 8 * 1024;

/// Headers the Agent's HTTP and MCP stacks own; a Controller value would override protocol state.
const RESERVED_HEADERS: [&str; 14] = [
    "host",
    "content-length",
    "content-type",
    "accept",
    "mcp-session-id",
    "mcp-protocol-version",
    "last-event-id",
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

/// One Controller-provided HTTP MCP server. There is no stdio form: the Node never runs a command
/// a Controller names.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct HostMcpServer {
    pub name: String,
    pub url: String,
    pub headers: HostMcpHeaders,
}

impl<'de> Deserialize<'de> for HostMcpServer {
    /// Accepts only a JSON object: a derived decoder would also read the fields from an array by
    /// position, and D1 defines each entry as an object.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// The object fields, with Serde's duplicate, missing and unknown field checks.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Fields {
            name: String,
            url: String,
            headers: HostMcpHeaders,
        }

        /// Refuses every non-object shape before the fields are read.
        struct ServerVisitor;

        impl<'de> Visitor<'de> for ServerVisitor {
            type Value = HostMcpServer;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a host MCP server object")
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                let Fields { name, url, headers } =
                    Fields::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(HostMcpServer { name, url, headers })
            }
        }

        deserializer.deserialize_map(ServerVisitor)
    }
}

impl fmt::Debug for HostMcpServer {
    /// The URL path and the headers may both carry the Controller's credential.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HostMcpServer")
            .field("name", &self.name)
            .field("url", &"[REDACTED]")
            .field("headers", &"[REDACTED]")
            .finish()
    }
}

/// Request headers of one host server. Decoding rejects a repeated JSON key instead of letting the
/// later value win, so the Node and a Controller can never disagree about which value was meant.
#[derive(Clone, Default, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct HostMcpHeaders(BTreeMap<String, String>);

impl HostMcpHeaders {
    /// Iterates in byte order of the header names.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }

    /// Number of headers.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the server sends no extra headers.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<(String, String)> for HostMcpHeaders {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<'de> Deserialize<'de> for HostMcpHeaders {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// Collects a JSON object while refusing duplicate keys.
        struct HeadersVisitor;

        impl<'de> Visitor<'de> for HeadersVisitor {
            type Value = HostMcpHeaders;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an object of header names to values")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut headers = BTreeMap::new();
                while let Some((name, value)) = map.next_entry::<String, String>()? {
                    if headers.contains_key(&name) {
                        return Err(serde::de::Error::custom("duplicate host MCP header"));
                    }
                    headers.insert(name, value);
                }
                Ok(HostMcpHeaders(headers))
            }
        }

        deserializer.deserialize_map(HeadersVisitor)
    }
}

/// Rejects every input outside the decision's grammar and bounds instead of normalizing it: a
/// Controller must mirror these rules exactly, and a disagreement closes the whole connection.
pub(crate) fn validate_host_servers(
    servers: &[HostMcpServer],
) -> Result<(), MessageValidationError> {
    let invalid = || MessageValidationError::InvalidHostMcpServers;
    if servers.len() > MAX_HOST_MCP_SERVERS {
        return Err(invalid());
    }
    let mut previous: Option<&str> = None;
    let mut total = 0usize;
    for server in servers {
        // Strictly increasing names are also unique, so no separate duplicate check is needed.
        if !is_host_name(&server.name)
            || previous.is_some_and(|previous| previous >= server.name.as_str())
        {
            return Err(invalid());
        }
        previous = Some(&server.name);
        if server.url.len() > MAX_HOST_MCP_URL_BYTES || !url::is_host_mcp_url(&server.url) {
            return Err(invalid());
        }
        if server.headers.len() > MAX_HOST_MCP_HEADERS
            || !server
                .headers
                .iter()
                .all(|(name, value)| is_header_name(name) && is_header_value(value))
        {
            return Err(invalid());
        }
        total += server.url.len()
            + server
                .headers
                .iter()
                .map(|(name, value)| name.len() + value.len())
                .sum::<usize>();
        if total > MAX_HOST_MCP_TOTAL_BYTES {
            return Err(invalid());
        }
    }
    Ok(())
}

/// `ora-` and at least one more byte, lowercase ASCII letters, digits and `-` only. Without `/`
/// a host name can never equal a canonical plugin MCP server name.
fn is_host_name(name: &str) -> bool {
    (5..=64).contains(&name.len())
        && name.starts_with("ora-")
        && name
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
}

/// A lowercase RFC 9110 token the Agent's HTTP and MCP stacks do not own.
fn is_header_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.bytes().all(|byte| {
            matches!(byte, b'a'..=b'z' | b'0'..=b'9') || b"!#$%&'*+-.^_`|~".contains(&byte)
        })
        && !RESERVED_HEADERS.contains(&name)
}

/// Visible ASCII and inner spaces, so no value can split or fold a header line.
fn is_header_value(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with(' ')
        && !value.ends_with(' ')
        && value.bytes().all(|byte| matches!(byte, b' '..=b'~'))
}
