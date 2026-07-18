use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::common::EnvVariable;

/// An HTTP header to set when making requests to the MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/mcp.ts")]
pub struct HttpHeader {
    /// The name of the HTTP header.
    pub name: String,
    /// The value to set for the HTTP header.
    pub value: String,
}

impl HttpHeader {
    /// Builds [`HttpHeader`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Selects exactly one MCP transport configuration shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export_to = "acp/mcp.ts")]
pub enum McpServer {
    /// HTTP transport configuration
    ///
    /// Only available when the Agent capabilities indicate `mcp_capabilities.http` is `true`.
    Http(McpServerHttp),
    /// SSE transport configuration
    ///
    /// Only available when the Agent capabilities indicate `mcp_capabilities.sse` is `true`.
    Sse(McpServerSse),
    /// Stdio transport configuration
    ///
    /// All Agents MUST support this transport.
    #[serde(untagged)]
    Stdio(McpServerStdio),
}

/// HTTP transport configuration for MCP.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/mcp.ts")]
pub struct McpServerHttp {
    /// Human-readable name identifying this MCP server.
    pub name: String,
    /// URL to the MCP server.
    pub url: String,
    /// HTTP headers to set when making requests to the MCP server.
    pub headers: Vec<HttpHeader>,
}

impl McpServerHttp {
    /// Builds [`McpServerHttp`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            headers: Vec::new(),
        }
    }

    /// HTTP headers to set when making requests to the MCP server.
    #[must_use]
    pub fn headers(mut self, headers: Vec<HttpHeader>) -> Self {
        self.headers = headers;
        self
    }
}

/// SSE transport configuration for MCP.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/mcp.ts")]
pub struct McpServerSse {
    /// Human-readable name identifying this MCP server.
    pub name: String,
    /// URL to the MCP server.
    pub url: String,
    /// HTTP headers to set when making requests to the MCP server.
    pub headers: Vec<HttpHeader>,
}

impl McpServerSse {
    /// Builds [`McpServerSse`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            url: url.into(),
            headers: Vec::new(),
        }
    }

    /// HTTP headers to set when making requests to the MCP server.
    #[must_use]
    pub fn headers(mut self, headers: Vec<HttpHeader>) -> Self {
        self.headers = headers;
        self
    }
}

/// Stdio transport configuration for MCP.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/mcp.ts")]
pub struct McpServerStdio {
    /// Human-readable name identifying this MCP server.
    pub name: String,
    /// Absolute path to the MCP server executable.
    pub command: PathBuf,
    /// Command-line arguments to pass to the MCP server.
    pub args: Vec<String>,
    /// Environment variables to set when launching the MCP server.
    pub env: Vec<EnvVariable>,
}

impl McpServerStdio {
    /// Builds [`McpServerStdio`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(name: impl Into<String>, command: impl Into<PathBuf>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    /// Command-line arguments to pass to the MCP server.
    #[must_use]
    pub fn args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Environment variables to set when launching the MCP server.
    #[must_use]
    pub fn env(mut self, env: Vec<EnvVariable>) -> Self {
        self.env = env;
        self
    }
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    HttpHeader::export(config)?;
    McpServer::export(config)?;
    McpServerHttp::export(config)?;
    McpServerSse::export(config)?;
    McpServerStdio::export(config)?;
    Ok(())
}
