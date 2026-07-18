// Generated from ora-plugin-protocol. Do not edit.

export type JsonValue = null | boolean | number | string | readonly JsonValue[] | { readonly [key: string]: JsonValue };

export type PluginId = string;

export type AgentProviderId = string;

export type PluginVersion = string;

export type PluginRelativePath = string;

export type ContentDigest = string;

export type ContentOwnerId = string;

export type OperationId = string;

export type CandidateAuditId = string;

export type AgentProviderKey = { pluginId: PluginId, providerId: AgentProviderId, };

export type PluginPackageManifest = { name: string, version: PluginVersion, type?: PackageModuleType, ora: PluginManifest, };

export type PackageModuleType = "module";

export type PluginManifest = { "kind": "agent", manifestVersion: number, id: PluginId, displayName: string, main: PluginRelativePath, engines: AgentEngines, contributes: AgentContributions, } | { "kind": "workbench", manifestVersion: number, id: PluginId, displayName: string, engines: WorkbenchEngines, contributes: WorkbenchContributions, };

export type PluginKind = "agent" | "workbench";

export type AgentEngines = { ora: EngineRange, pluginApi: number, bun: EngineRange, };

export type WorkbenchEngines = { ora: EngineRange, };

export type EngineRange = string;

export type AgentContributions = { agents: Array<AgentContribution>, };

export type AgentContribution = { id: AgentProviderId, displayName: string, contractVersion: number, };

export type WorkbenchContributions = { workbench: WorkbenchContribution, };

export type WorkbenchContribution = { schemaVersion: number, };

export type AgentInstallationId = string;

export type AgentConversationId = string;

export type AgentTurnId = string;

export type AgentCursor = string;

export type AgentResourceId = string;

export type AgentToolCallId = string;

export type ProjectHandle = string;

export type WorktreeHandle = string;

export type AgentConfigurationKey = string;

export type ClientRequestId = string;

export type HostResolvedAbsolutePath = string;

export type AgentPrompt = string;

export type Rfc3339Timestamp = string;

export type JsonSafeU64 = number;

export type AgentPageLimit = number;

export type FiniteJsonNumber = number;

export type AgentScope = { "type": "global", } | { "type": "project", projectHandle: ProjectHandle, workingDirectory: HostResolvedAbsolutePath, } | { "type": "worktree", projectHandle: ProjectHandle, worktreeHandle: WorktreeHandle, workingDirectory: HostResolvedAbsolutePath, };

export type DiscoverInstallationsRequest = { providerId: AgentProviderId, scope: AgentScope, };

export type GetConfigurationSummaryRequest = { providerId: AgentProviderId, installationId: AgentInstallationId, scope: AgentScope, };

export type ListSkillsRequest = { providerId: AgentProviderId, installationId: AgentInstallationId, scope: AgentScope, cursor?: AgentCursor, limit: AgentPageLimit, };

export type ListMcpServersRequest = { providerId: AgentProviderId, installationId: AgentInstallationId, scope: AgentScope, cursor?: AgentCursor, limit: AgentPageLimit, };

export type ListConversationsRequest = { providerId: AgentProviderId, installationId: AgentInstallationId, scope: AgentScope, cursor?: AgentCursor, limit: AgentPageLimit, };

export type StartConversationRequest = { providerId: AgentProviderId, installationId: AgentInstallationId, scope: AgentScope, clientRequestId: ClientRequestId, prompt: AgentPrompt, };

export type SendMessageRequest = { providerId: AgentProviderId, installationId: AgentInstallationId, conversationId: AgentConversationId, scope: AgentScope, clientRequestId: ClientRequestId, prompt: AgentPrompt, };

export type CancelConversationRequest = { providerId: AgentProviderId, installationId: AgentInstallationId, conversationId: AgentConversationId, scope: AgentScope, };

export type DiscoverInstallationsResponse = { installations: Array<AgentInstallation>, diagnostics: Array<AgentDiscoveryDiagnostic>, };

export type AgentInstallation = { installationId: AgentInstallationId, displayName: string, version?: string, locationDisplay?: string, availability: AgentAvailability, };

export type AgentAvailability = { "type": "available", } | { "type": "unavailable", reason: string, };

export type AgentDiscoveryDiagnostic = { kind: AgentDiscoveryDiagnosticKind, message: string, };

export type AgentDiscoveryDiagnosticKind = "notFound" | "permissionDenied" | "probeFailed";

export type GetConfigurationSummaryResponse = { items: Array<AgentConfigurationItem>, };

export type AgentConfigurationItem = { key: AgentConfigurationKey, displayName: string, source: AgentResourceSource, value: AgentConfigurationValue, };

export type AgentConfigurationValue = { "type": "unset", } | { "type": "redacted", } | { "type": "boolean", value: boolean, } | { "type": "number", value: FiniteJsonNumber, } | { "type": "string", value: string, } | { "type": "stringList", value: Array<string>, };

export type ListSkillsResponse = { items: Array<AgentSkillSummary>, nextCursor?: AgentCursor, };

export type AgentSkillSummary = { id: AgentResourceId, displayName: string, description?: string, source: AgentResourceSource, };

export type ListMcpServersResponse = { items: Array<AgentMcpServerSummary>, nextCursor?: AgentCursor, };

export type AgentMcpServerSummary = { id: AgentResourceId, displayName: string, transport: AgentMcpTransport, enabled: boolean, source: AgentResourceSource, };

export type AgentResourceSource = { "type": "user", } | { "type": "project", } | { "type": "worktree", } | { "type": "builtIn", } | { "type": "unknown", display?: string, };

export type AgentMcpTransport = "stdio" | "http" | "sse" | "unknown";

export type ListConversationsResponse = { items: Array<AgentConversationSummary>, nextCursor?: AgentCursor, };

export type AgentConversationSummary = { conversationId: AgentConversationId, title?: string, updatedAt?: Rfc3339Timestamp, };

export type AgentEvent = { "kind": "conversationStarted", conversationId: AgentConversationId, } | { "kind": "textDelta", channel: AgentOutputChannel, text: string, } | { "kind": "status", phase: string, message?: string, } | { "kind": "toolCall", callId: AgentToolCallId, name: string, summary?: string, } | { "kind": "toolResult", callId: AgentToolCallId, isError: boolean, summary?: string, } | { "kind": "usage", usage: AgentUsage, };

export type AgentOutputChannel = "assistant" | "reasoning" | "tool";

export type AgentUsage = { inputTokens?: JsonSafeU64, outputTokens?: JsonSafeU64, costMicros?: JsonSafeU64, };

export type AgentTurnResult = { conversationId: AgentConversationId, turnId?: AgentTurnId, finishReason: AgentFinishReason, usage?: AgentUsage, };

export type AgentFinishReason = "completed" | "cancelled" | "limit";

export type CancelConversationResponse = { disposition: CancelDisposition, };

export type CancelDisposition = "accepted" | "alreadyStopped";

export type AgentMethod = "agent.discoverInstallations" | "agent.getConfigurationSummary" | "agent.listSkills" | "agent.listMcpServers" | "agent.listConversations" | "agent.startConversation" | "agent.sendMessage" | "agent.cancelConversation";

export type InvocationSemantics = "idempotent" | "nonIdempotent";

export type AgentBusinessFailureKind = "agentUnavailable" | "authenticationRequired" | "invalidAgentConfiguration" | "installationNotFound" | "conversationNotFound" | "unsupportedAgentCapability" | "invalidState" | "permissionDenied" | "cursorExpired" | "agentProcessFailed" | "providerFailure";

export type AgentBusinessErrorData = { kind: AgentBusinessFailureKind, retryable: boolean, details?: Record<string, JsonValue>, };

export type InitializeParams = { wireVersion: number, hostVersion: PluginVersion, runtimeVersion: PluginVersion, sessionId: string, plugin: InitializePlugin, paths: InitializePaths, declaredAgents: Array<DeclaredAgent>, limits: InitializeLimits, };

export type InitializePlugin = { id: PluginId, version: PluginVersion, kind: PluginKind, pluginApi: number, contentOwner: ContentOwnerId, };

export type InitializePaths = { extensionPath: HostResolvedAbsolutePath, entryPath: HostResolvedAbsolutePath, storagePath: HostResolvedAbsolutePath, };

export type DeclaredAgent = { id: AgentProviderId, contractVersion: number, };

export type InitializeLimits = { maxFrameBytes: number, maxPendingRequests: number, maxAgentEventBytes: number, maxAgentResultBytes: number, maxAgentPromptBytes: number, maxActiveTurns: number, maxPageItems: number, };

export type InitializeResult = { wireVersion: number, runtimeVersion: PluginVersion, sessionId: string, plugin: InitializeResultPlugin, };

export type InitializeResultPlugin = { id: PluginId, version: PluginVersion, };

export type ActivationReason = "lazyInvocation" | "manualStart";

export type ActivateParams = { reason: ActivationReason, };

export type ActivateResult = { providers: Array<DeclaredAgent>, };

export type DeactivationReason = "manualStop" | "disable" | "uninstall" | "shutdown" | "grantChanged";

export type DeactivateParams = { reason: DeactivationReason, };

export type CancelRequestParams = { id: string, };

export type StreamParams = { id: string, seq: JsonSafeU64, value: AgentEvent, };
