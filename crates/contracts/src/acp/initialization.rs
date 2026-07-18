use super::{authentication::AuthMethod, common::EmptyObject, serde_util::IntoOption};
use serde::{Deserialize, Serialize};
use serde_with::{DefaultOnError, serde_as, skip_serializing_none};
use ts_rs::TS;

/// Identifies the ACP version negotiated between a client and an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(transparent)]
#[ts(export_to = "acp/initialization.ts")]
pub struct ProtocolVersion(pub u16);

/// Request parameters for the initialize method.
///
/// Sent by the client to establish connection and negotiate capabilities.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/initialization.ts")]
pub struct InitializeRequest {
    /// The latest protocol version supported by the client.
    pub protocol_version: ProtocolVersion,
    /// Capabilities supported by the client.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub client_capabilities: ClientCapabilities,
    /// Information about the Client name and version sent to the Agent.
    ///
    /// Note: in future versions of the protocol, this will be required.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub client_info: Option<Implementation>,
}

impl InitializeRequest {
    /// Builds [`InitializeRequest`] with the required request fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(protocol_version: ProtocolVersion) -> Self {
        Self {
            protocol_version,
            client_capabilities: ClientCapabilities::default(),
            client_info: None,
        }
    }

    /// Capabilities supported by the client.
    #[must_use]
    pub fn client_capabilities(mut self, client_capabilities: ClientCapabilities) -> Self {
        self.client_capabilities = client_capabilities;
        self
    }

    /// Information about the Client name and version sent to the Agent.
    #[must_use]
    pub fn client_info(mut self, client_info: impl IntoOption<Implementation>) -> Self {
        self.client_info = client_info.into_option();
        self
    }
}

/// Response to the `initialize` method.
///
/// Contains the negotiated protocol version and agent capabilities.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/initialization.ts")]
pub struct InitializeResponse {
    /// The protocol version the client specified if supported by the agent,
    /// or the latest protocol version supported by the agent.
    ///
    /// The client should disconnect, if it doesn't support this version.
    pub protocol_version: ProtocolVersion,
    /// Capabilities supported by the agent.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub agent_capabilities: AgentCapabilities,
    /// Authentication methods supported by the agent.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub auth_methods: Vec<AuthMethod>,
    /// Information about the Agent name and version sent to the Client.
    ///
    /// Note: in future versions of the protocol, this will be required.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub agent_info: Option<Implementation>,
}

impl InitializeResponse {
    /// Builds [`InitializeResponse`] with the required response fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(protocol_version: ProtocolVersion) -> Self {
        Self {
            protocol_version,
            agent_capabilities: AgentCapabilities::default(),
            auth_methods: vec![],
            agent_info: None,
        }
    }

    /// Capabilities supported by the agent.
    #[must_use]
    pub fn agent_capabilities(mut self, agent_capabilities: AgentCapabilities) -> Self {
        self.agent_capabilities = agent_capabilities;
        self
    }

    /// Authentication methods supported by the agent.
    #[must_use]
    pub fn auth_methods(mut self, auth_methods: Vec<AuthMethod>) -> Self {
        self.auth_methods = auth_methods;
        self
    }

    /// Information about the Agent name and version sent to the Client.
    #[must_use]
    pub fn agent_info(mut self, agent_info: impl IntoOption<Implementation>) -> Self {
        self.agent_info = agent_info.into_option();
        self
    }
}

/// Capabilities supported by the client.
///
/// Advertised during initialization to inform the agent about
/// available features and methods.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/initialization.ts")]
pub struct ClientCapabilities {
    /// File system capabilities supported by the client.
    /// Determines which file operations the agent can request.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub fs: FileSystemCapabilities,
    /// Whether the Client support all `terminal/*` methods.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub terminal: bool,
    /// Session-related capabilities supported by the client.
    ///
    /// Optional. Omitted or `null` both mean the client does not advertise any
    /// session-related extensions.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub session: Option<ClientSessionCapabilities>,
    ///// This capability is not part of the spec yet, and may be removed or changed at any point.
    /////
    ///// Whether the client supports `plan_update` and `plan_removed` session updates.
    /////
    ///// Optional. Omitted or `null` both mean the client does not advertise support.
    ///// Supplying `{}` means the client can receive both update types.
    // #[serde_as(deserialize_as = "DefaultOnError")]
    // #[serde(default)]
    // pub plan: Option<PlanCapabilities>,
}

impl ClientCapabilities {
    /// Builds an empty [`ClientCapabilities`]; use builder methods to advertise supported sub-capabilities.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// File system capabilities supported by the client.
    /// Determines which file operations the agent can request.
    #[must_use]
    pub fn fs(mut self, fs: FileSystemCapabilities) -> Self {
        self.fs = fs;
        self
    }

    /// Whether the Client support all `terminal/*` methods.
    #[must_use]
    pub fn terminal(mut self, terminal: bool) -> Self {
        self.terminal = terminal;
        self
    }

    /// Session-related capabilities supported by the client.
    #[must_use]
    pub fn session(mut self, session: impl IntoOption<ClientSessionCapabilities>) -> Self {
        self.session = session.into_option();
        self
    }
}

/// Session-related capabilities supported by the client.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/initialization.ts")]
pub struct ClientSessionCapabilities {
    /// Config option capabilities supported by the client.
    ///
    /// Omitted or `null` both mean the client does not advertise support for any
    /// config option extensions.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub config_options: Option<SessionConfigOptionsCapabilities>,
}

impl ClientSessionCapabilities {
    /// Builds an empty [`ClientSessionCapabilities`]; use builder methods to advertise supported sub-capabilities.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Config option capabilities supported by the client.
    ///
    /// Omitted or `null` both mean the client does not advertise support for any
    /// config option extensions.
    #[must_use]
    pub fn config_options(
        mut self,
        config_options: impl IntoOption<SessionConfigOptionsCapabilities>,
    ) -> Self {
        self.config_options = config_options.into_option();
        self
    }
}

/// Session configuration option capabilities supported by the client.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/initialization.ts")]
pub struct SessionConfigOptionsCapabilities {
    /// Whether the client supports boolean session configuration options.
    ///
    /// Optional. Omitted or `null` both mean the client does not advertise support.
    /// Supplying `{}` means agents may include `type: "boolean"` entries in
    /// `configOptions`, and the client may send `session/set_config_option`
    /// requests with `type: "boolean"` and a boolean `value`.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub boolean: Option<EmptyObject>,
}

impl SessionConfigOptionsCapabilities {
    /// Builds an empty [`SessionConfigOptionsCapabilities`]; use builder methods to advertise supported sub-capabilities.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the client supports boolean session configuration options.
    ///
    /// Omitted or `null` both mean the client does not advertise support.
    /// Supplying `{}` means agents may include `type: "boolean"` entries in
    /// `configOptions`, and the client may send `session/set_config_option`
    /// requests with `type: "boolean"` and a boolean `value`.
    #[must_use]
    pub fn boolean(mut self, boolean: impl IntoOption<EmptyObject>) -> Self {
        self.boolean = boolean.into_option();
        self
    }
}

/// File system capabilities that a client may support.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/initialization.ts")]
pub struct FileSystemCapabilities {
    /// Whether the Client supports `fs/read_text_file` requests.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub read_text_file: bool,
    /// Whether the Client supports `fs/write_text_file` requests.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub write_text_file: bool,
}

impl FileSystemCapabilities {
    /// Builds an empty [`FileSystemCapabilities`]; use builder methods to advertise supported sub-capabilities.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the Client supports `fs/read_text_file` requests.
    #[must_use]
    pub fn read_text_file(mut self, read_text_file: bool) -> Self {
        self.read_text_file = read_text_file;
        self
    }

    /// Whether the Client supports `fs/write_text_file` requests.
    #[must_use]
    pub fn write_text_file(mut self, write_text_file: bool) -> Self {
        self.write_text_file = write_text_file;
        self
    }
}

/// Capabilities supported by the agent.
///
/// Advertised during initialization to inform the client about
/// available features and content types.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/initialization.ts")]
pub struct AgentCapabilities {
    /// Whether the agent supports `session/load`.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub load_session: bool,
    /// Prompt capabilities supported by the agent.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub prompt_capabilities: PromptCapabilities,
    /// MCP capabilities supported by the agent.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub mcp_capabilities: McpCapabilities,
    /// Session lifecycle and prompt capabilities advertised by the agent.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub session_capabilities: SessionCapabilities,
    /// Authentication-related capabilities supported by the agent.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub auth: AgentAuthCapabilities,
}

impl AgentCapabilities {
    /// Builds an empty [`AgentCapabilities`]; use builder methods to advertise supported sub-capabilities.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the agent supports `session/load`.
    #[must_use]
    pub fn load_session(mut self, load_session: bool) -> Self {
        self.load_session = load_session;
        self
    }

    /// Prompt capabilities supported by the agent.
    #[must_use]
    pub fn prompt_capabilities(mut self, prompt_capabilities: PromptCapabilities) -> Self {
        self.prompt_capabilities = prompt_capabilities;
        self
    }

    /// MCP capabilities supported by the agent.
    #[must_use]
    pub fn mcp_capabilities(mut self, mcp_capabilities: McpCapabilities) -> Self {
        self.mcp_capabilities = mcp_capabilities;
        self
    }

    /// Session capabilities supported by the agent.
    #[must_use]
    pub fn session_capabilities(mut self, session_capabilities: SessionCapabilities) -> Self {
        self.session_capabilities = session_capabilities;
        self
    }

    /// Authentication-related capabilities supported by the agent.
    #[must_use]
    pub fn auth(mut self, auth: AgentAuthCapabilities) -> Self {
        self.auth = auth;
        self
    }
}

/// Prompt capabilities supported by the agent in `session/prompt` requests.
///
/// Baseline agent functionality requires support for [`ContentBlock::Text`]
/// and [`ContentBlock::ResourceLink`] in prompt requests.
///
/// Other variants must be explicitly opted in to.
/// Capabilities for different types of content in prompt requests.
///
/// Indicates which content types beyond the baseline (text and resource links)
/// the agent can process.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/initialization.ts")]
pub struct PromptCapabilities {
    /// Agent supports [`ContentBlock::Image`].
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub image: bool,
    /// Agent supports [`ContentBlock::Audio`].
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub audio: bool,
    /// Agent supports embedded context in `session/prompt` requests.
    ///
    /// When enabled, the Client is allowed to include [`ContentBlock::Resource`]
    /// in prompt requests for pieces of context that are referenced in the message.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub embedded_context: bool,
}

impl PromptCapabilities {
    /// Builds an empty [`PromptCapabilities`]; use builder methods to advertise supported sub-capabilities.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Agent supports [`ContentBlock::Image`].
    #[must_use]
    pub fn image(mut self, image: bool) -> Self {
        self.image = image;
        self
    }

    /// Agent supports [`ContentBlock::Audio`].
    #[must_use]
    pub fn audio(mut self, audio: bool) -> Self {
        self.audio = audio;
        self
    }

    /// Agent supports embedded context in `session/prompt` requests.
    ///
    /// When enabled, the Client is allowed to include [`ContentBlock::Resource`]
    /// in prompt requests for pieces of context that are referenced in the message.
    #[must_use]
    pub fn embedded_context(mut self, embedded_context: bool) -> Self {
        self.embedded_context = embedded_context;
        self
    }
}

/// MCP capabilities supported by the agent
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/initialization.ts")]
pub struct McpCapabilities {
    /// Agent supports [`McpServer::Http`].
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub http: bool,
    /// Agent supports [`McpServer::Sse`].
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub sse: bool,
}

impl McpCapabilities {
    /// Builds an empty [`McpCapabilities`]; use builder methods to advertise supported sub-capabilities.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Agent supports [`McpServer::Http`].
    #[must_use]
    pub fn http(mut self, http: bool) -> Self {
        self.http = http;
        self
    }

    /// Agent supports [`McpServer::Sse`].
    #[must_use]
    pub fn sse(mut self, sse: bool) -> Self {
        self.sse = sse;
        self
    }
}

/// Session capabilities supported by the agent.
///
/// As a baseline, all Agents **MUST** support `session/new`, `session/prompt`, `session/cancel`, and `session/update`.
///
/// Optionally, they **MAY** support other session methods and notifications by specifying additional capabilities.
///
/// Note: `session/load` is still handled by the top-level `load_session` capability. This will be unified in future versions of the protocol.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/initialization.ts")]
pub struct SessionCapabilities {
    /// Whether the agent supports `session/list`.
    ///
    /// Optional. Omitted or `null` both mean the agent does not advertise support.
    /// Supplying `{}` means the agent supports listing sessions.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub list: Option<EmptyObject>,
    /// Whether the agent supports `session/delete`.
    ///
    /// Optional. Omitted or `null` both mean the agent does not advertise support.
    /// Supplying `{}` means the agent supports deleting sessions from `session/list`.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub delete: Option<EmptyObject>,
    /// Whether the agent supports `additionalDirectories` on supported session lifecycle requests.
    ///
    /// Optional. Omitted or `null` both mean the agent does not advertise support.
    /// Supplying `{}` means the agent supports `additionalDirectories` on
    /// supported session lifecycle requests.
    ///
    /// Agents that also support `session/list` may return
    /// `SessionInfo.additionalDirectories` to report the complete ordered
    /// additional-root list associated with a listed session.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub additional_directories: Option<EmptyObject>,
    // /// This capability is not part of the spec yet, and may be removed or changed at any point.
    // ///
    // /// Whether the agent supports `session/fork`.
    // ///
    // /// Optional. Omitted or `null` both mean the agent does not advertise support.
    // /// Supplying `{}` means the agent supports forking sessions.
    // #[serde_as(deserialize_as = "DefaultOnError")]
    // #[serde(default)]
    // pub fork: Option<EmptyObject>,
    /// Whether the agent supports `session/resume`.
    ///
    /// Optional. Omitted or `null` both mean the agent does not advertise support.
    /// Supplying `{}` means the agent supports resuming sessions.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub resume: Option<EmptyObject>,
    /// Whether the agent supports `session/close`.
    ///
    /// Optional. Omitted or `null` both mean the agent does not advertise support.
    /// Supplying `{}` means the agent supports closing sessions.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub close: Option<EmptyObject>,
}

impl SessionCapabilities {
    /// Builds an empty [`SessionCapabilities`]; use builder methods to advertise supported sub-capabilities.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the agent supports `session/list`.
    ///
    /// Omitted or `null` both mean the agent does not advertise support.
    /// Supplying `{}` means the agent supports listing sessions.
    #[must_use]
    pub fn list(mut self, list: impl IntoOption<EmptyObject>) -> Self {
        self.list = list.into_option();
        self
    }

    /// Whether the agent supports `session/delete`.
    ///
    /// Omitted or `null` both mean the agent does not advertise support.
    /// Supplying `{}` means the agent supports deleting sessions from `session/list`.
    #[must_use]
    pub fn delete(mut self, delete: impl IntoOption<EmptyObject>) -> Self {
        self.delete = delete.into_option();
        self
    }

    /// Whether the agent supports `additionalDirectories` on supported session lifecycle requests.
    ///
    /// Omitted or `null` both mean the agent does not advertise support.
    /// Supplying `{}` means the agent supports `additionalDirectories` on
    /// supported session lifecycle requests.
    ///
    /// Agents that also support `session/list` may return
    /// `SessionInfo.additionalDirectories` to report the complete ordered
    /// additional-root list associated with a listed session.
    #[must_use]
    pub fn additional_directories(
        mut self,
        additional_directories: impl IntoOption<EmptyObject>,
    ) -> Self {
        self.additional_directories = additional_directories.into_option();
        self
    }

    // /// Whether the agent supports `session/fork`.
    // ///
    // /// Omitted or `null` both mean the agent does not advertise support.
    // /// Supplying `{}` means the agent supports forking sessions.
    // #[must_use]
    // pub fn fork(mut self, fork: impl IntoOption<SessionForkCapabilities>) -> Self {
    //     self.fork = fork.into_option();
    //     self
    // }

    /// Whether the agent supports `session/resume`.
    ///
    /// Omitted or `null` both mean the agent does not advertise support.
    /// Supplying `{}` means the agent supports resuming sessions.
    #[must_use]
    pub fn resume(mut self, resume: impl IntoOption<EmptyObject>) -> Self {
        self.resume = resume.into_option();
        self
    }

    /// Whether the agent supports `session/close`.
    ///
    /// Omitted or `null` both mean the agent does not advertise support.
    /// Supplying `{}` means the agent supports closing sessions.
    #[must_use]
    pub fn close(mut self, close: impl IntoOption<EmptyObject>) -> Self {
        self.close = close.into_option();
        self
    }
}

/// Authentication-related capabilities supported by the agent.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/initialization.ts")]
pub struct AgentAuthCapabilities {
    /// Whether the agent supports the logout method.
    ///
    /// Optional. Omitted or `null` both mean the agent does not advertise support.
    /// Supplying `{}` means the agent supports the logout method.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub logout: Option<EmptyObject>,
}

impl AgentAuthCapabilities {
    /// Builds an empty [`AgentAuthCapabilities`]; use builder methods to advertise supported sub-capabilities.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the agent supports the logout method.
    #[must_use]
    pub fn logout(mut self, logout: impl IntoOption<EmptyObject>) -> Self {
        self.logout = logout.into_option();
        self
    }
}

/// Metadata about the implementation of the client or agent.
/// Describes the name and version of an ACP implementation, with an optional
/// title for UI representation.
#[serde_as]
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "acp/initialization.ts")]
pub struct Implementation {
    /// Intended for programmatic or logical use, but can be used as a display
    /// name fallback if title isn’t present.
    pub name: String,
    /// Intended for UI and end-user contexts — optimized to be human-readable
    /// and easily understood.
    ///
    /// If not provided, the name should be used for display.
    #[serde_as(deserialize_as = "DefaultOnError")]
    #[serde(default)]
    pub title: Option<String>,
    /// Version of the implementation. Can be displayed to the user or used
    /// for debugging or metrics purposes. (e.g. "1.0.0").
    pub version: String,
}

impl Implementation {
    /// Builds [`Implementation`] with the required fields set; optional fields start unset or empty.
    #[must_use]
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            title: None,
            version: version.into(),
        }
    }

    /// Intended for UI and end-user contexts — optimized to be human-readable
    /// and easily understood.
    ///
    /// If not provided, the name should be used for display.
    #[must_use]
    pub fn title(mut self, title: impl IntoOption<String>) -> Self {
        self.title = title.into_option();
        self
    }
}

/// Exports every TypeScript binding declared in this module into the target directory.
pub(crate) fn export(config: &ts_rs::Config) -> Result<(), ts_rs::ExportError> {
    ProtocolVersion::export(config)?;
    InitializeRequest::export(config)?;
    InitializeResponse::export(config)?;
    ClientCapabilities::export(config)?;
    ClientSessionCapabilities::export(config)?;
    SessionConfigOptionsCapabilities::export(config)?;
    FileSystemCapabilities::export(config)?;
    AgentCapabilities::export(config)?;
    PromptCapabilities::export(config)?;
    McpCapabilities::export(config)?;
    SessionCapabilities::export(config)?;
    AgentAuthCapabilities::export(config)?;
    Implementation::export(config)?;
    Ok(())
}
