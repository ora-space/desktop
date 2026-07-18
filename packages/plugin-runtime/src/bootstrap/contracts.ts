import type {
  AgentEvent,
  AgentTurnResult,
  InitializeLimits,
  InitializeParams,
} from "../generated/index.js";
import {
  requireBoundedString,
  requireExactKeys,
  requirePlainObject,
  utf8Bytes,
} from "../rpc/envelope.js";

export const AGENT_METHODS = [
  "agent.discoverInstallations",
  "agent.getConfigurationSummary",
  "agent.listSkills",
  "agent.listMcpServers",
  "agent.listConversations",
  "agent.startConversation",
  "agent.sendMessage",
  "agent.cancelConversation",
] as const;

export type AgentMethodName = (typeof AGENT_METHODS)[number];

const ORDINARY_METHODS = new Set<AgentMethodName>([
  "agent.discoverInstallations",
  "agent.getConfigurationSummary",
  "agent.listSkills",
  "agent.listMcpServers",
  "agent.listConversations",
  "agent.startConversation",
  "agent.sendMessage",
]);

export function isAgentMethod(method: string): method is AgentMethodName {
  return (AGENT_METHODS as readonly string[]).includes(method);
}

export function isOrdinaryMethod(method: AgentMethodName): boolean {
  return ORDINARY_METHODS.has(method);
}

/** Validates the exact initialize object before any plugin entry is imported. */
export function validateInitializeParams(value: unknown): InitializeParams {
  const root = requirePlainObject(value, "initialize params");
  requireExactKeys(root, [
    "wireVersion",
    "hostVersion",
    "runtimeVersion",
    "sessionId",
    "plugin",
    "paths",
    "declaredAgents",
    "limits",
  ]);
  if (root.wireVersion !== 1) {
    throw new TypeError("wireVersion must equal 1");
  }
  const hostVersion = requireVersion(root.hostVersion, "hostVersion");
  const runtimeVersion = requireVersion(root.runtimeVersion, "runtimeVersion");
  const sessionId = requireBoundedString(root.sessionId, "sessionId", 128);

  const plugin = requirePlainObject(root.plugin, "initialize plugin");
  requireExactKeys(plugin, ["id", "version", "kind", "pluginApi", "contentOwner"]);
  const pluginId = requirePluginId(plugin.id);
  const pluginVersion = requireVersion(plugin.version, "plugin.version");
  if (plugin.kind !== "agent" || plugin.pluginApi !== 1) {
    throw new TypeError("initialize plugin kind/pluginApi mismatch");
  }
  const contentOwner = requireBoundedString(plugin.contentOwner, "contentOwner", 96);

  const paths = requirePlainObject(root.paths, "initialize paths");
  requireExactKeys(paths, ["extensionPath", "entryPath", "storagePath"]);
  const extensionPath = requireAbsoluteWindowsPath(paths.extensionPath, "extensionPath");
  const entryPath = requireAbsoluteWindowsPath(paths.entryPath, "entryPath");
  const storagePath = requireAbsoluteWindowsPath(paths.storagePath, "storagePath");
  requireDescendantPath(extensionPath, entryPath);

  if (!Array.isArray(root.declaredAgents) || root.declaredAgents.length === 0 || root.declaredAgents.length > 64) {
    throw new TypeError("declaredAgents must contain 1..=64 descriptors");
  }
  const seen = new Set<string>();
  const declaredAgents = root.declaredAgents.map((candidate) => {
    const agent = requirePlainObject(candidate, "declared Agent");
    requireExactKeys(agent, ["id", "contractVersion"]);
    const id = requireProviderId(agent.id);
    if (agent.contractVersion !== 1 || seen.has(id)) {
      throw new TypeError("declared Agent descriptors must be unique Agent v1 providers");
    }
    seen.add(id);
    return { id, contractVersion: 1 as const };
  });
  const limits = validateInitializeLimits(root.limits);

  return {
    wireVersion: 1,
    hostVersion,
    runtimeVersion,
    sessionId,
    plugin: {
      id: pluginId,
      version: pluginVersion,
      kind: "agent",
      pluginApi: 1,
      contentOwner,
    },
    paths: { extensionPath, entryPath, storagePath },
    declaredAgents,
    limits,
  };
}

export function validateActivateParams(value: unknown): "lazyInvocation" | "manualStart" {
  const object = requirePlainObject(value, "activate params");
  requireExactKeys(object, ["reason"]);
  if (object.reason !== "lazyInvocation" && object.reason !== "manualStart") {
    throw new TypeError("activate reason is invalid");
  }
  return object.reason;
}

export function validateDeactivateParams(value: unknown): void {
  const object = requirePlainObject(value, "deactivate params");
  requireExactKeys(object, ["reason"]);
  if (!["manualStop", "disable", "uninstall", "shutdown", "grantChanged"].includes(String(object.reason))) {
    throw new TypeError("deactivate reason is invalid");
  }
}

export function validateCancelRequestParams(value: unknown): string {
  const object = requirePlainObject(value, "cancel request params");
  requireExactKeys(object, ["id"]);
  return requireHostRequestId(object.id);
}

/** Validates the common request leaves and exact method-specific key set before dispatch. */
export function validateAgentRequest(
  method: AgentMethodName,
  value: unknown,
  limits: InitializeLimits,
): Record<string, unknown> {
  const object = requirePlainObject(value, `${method} params`);
  const pageKeys = ["providerId", "installationId", "scope", "cursor", "limit"];
  switch (method) {
    case "agent.discoverInstallations":
      requireExactKeys(object, ["providerId", "scope"]);
      break;
    case "agent.getConfigurationSummary":
      requireExactKeys(object, ["providerId", "installationId", "scope"]);
      break;
    case "agent.listSkills":
    case "agent.listMcpServers":
    case "agent.listConversations":
      requireExactKeys(object, pageKeys, ["cursor"]);
      if (requirePageLimit(object.limit) > limits.maxPageItems) {
        throw new TypeError("page limit exceeds the negotiated maximum");
      }
      if (Object.hasOwn(object, "cursor")) {
        requireOpaqueId(object.cursor, "cursor");
      }
      break;
    case "agent.startConversation":
      requireExactKeys(object, ["providerId", "installationId", "scope", "clientRequestId", "prompt"]);
      requireClientRequestId(object.clientRequestId);
      requirePrompt(object.prompt, limits.maxAgentPromptBytes);
      break;
    case "agent.sendMessage":
      requireExactKeys(object, [
        "providerId",
        "installationId",
        "conversationId",
        "scope",
        "clientRequestId",
        "prompt",
      ]);
      requireOpaqueId(object.conversationId, "conversationId");
      requireClientRequestId(object.clientRequestId);
      requirePrompt(object.prompt, limits.maxAgentPromptBytes);
      break;
    case "agent.cancelConversation":
      requireExactKeys(object, ["providerId", "installationId", "conversationId", "scope"]);
      requireOpaqueId(object.conversationId, "conversationId");
      break;
  }
  requireProviderId(object.providerId);
  if (method !== "agent.discoverInstallations") {
    requireOpaqueId(object.installationId, "installationId");
  }
  validateScope(object.scope);
  return object;
}

/** Validates one stream event and enforces the negotiated event byte cap. */
export function validateAgentEvent(value: unknown, limits: InitializeLimits): AgentEvent {
  const event = requirePlainObject(value, "Agent event");
  const kind = event.kind;
  switch (kind) {
    case "conversationStarted":
      requireExactKeys(event, ["kind", "conversationId"]);
      requireOpaqueId(event.conversationId, "conversationId");
      break;
    case "textDelta":
      requireExactKeys(event, ["kind", "channel", "text"]);
      if (!["assistant", "reasoning", "tool"].includes(String(event.channel))) {
        throw new TypeError("textDelta channel is invalid");
      }
      requireDisplayString(event.text, "textDelta text", 256 * 1024);
      break;
    case "status":
      requireExactKeys(event, ["kind", "phase", "message"], ["message"]);
      requireDisplayString(event.phase, "status phase", 512);
      requireOptionalDisplayString(event, "message", 8192);
      break;
    case "toolCall":
      requireExactKeys(event, ["kind", "callId", "name", "summary"], ["summary"]);
      requireOpaqueId(event.callId, "callId");
      requireDisplayString(event.name, "tool name", 512);
      requireOptionalDisplayString(event, "summary", 8192);
      break;
    case "toolResult":
      requireExactKeys(event, ["kind", "callId", "isError", "summary"], ["summary"]);
      requireOpaqueId(event.callId, "callId");
      if (typeof event.isError !== "boolean") {
        throw new TypeError("toolResult isError must be boolean");
      }
      requireOptionalDisplayString(event, "summary", 8192);
      break;
    case "usage":
      requireExactKeys(event, ["kind", "usage"]);
      validateUsage(event.usage);
      break;
    default:
      throw new TypeError("unknown Agent event kind");
  }
  enforceEncodedLimit(event, limits.maxAgentEventBytes, "Agent event");
  return event as unknown as AgentEvent;
}

/** Validates the terminal generator return and negotiated result cap. */
export function validateTurnResult(value: unknown, limits: InitializeLimits): AgentTurnResult {
  const result = requirePlainObject(value, "Agent turn result");
  requireExactKeys(result, ["conversationId", "turnId", "finishReason", "usage"], ["turnId", "usage"]);
  requireOpaqueId(result.conversationId, "conversationId");
  if (Object.hasOwn(result, "turnId")) {
    requireOpaqueId(result.turnId, "turnId");
  }
  if (!["completed", "cancelled", "limit"].includes(String(result.finishReason))) {
    throw new TypeError("Agent finishReason is invalid");
  }
  if (Object.hasOwn(result, "usage")) {
    validateUsage(result.usage);
  }
  enforceEncodedLimit(result, limits.maxAgentResultBytes, "Agent turn result");
  return result as unknown as AgentTurnResult;
}

/** Applies top-level method result shape and byte/item caps before stdout serialization. */
export function validateAgentResult(
  method: AgentMethodName,
  value: unknown,
  limits: InitializeLimits,
  requestedLimit?: number,
): Record<string, unknown> {
  const result = requirePlainObject(value, `${method} result`);
  switch (method) {
    case "agent.discoverInstallations":
      requireExactKeys(result, ["installations", "diagnostics"]);
      validateDiscoveryResult(result);
      break;
    case "agent.getConfigurationSummary":
      requireExactKeys(result, ["items"]);
      validateConfigurationResult(result);
      break;
    case "agent.listSkills":
      requireExactKeys(result, ["items", "nextCursor"], ["nextCursor"]);
      validateSkillsResult(result, requestedLimit ?? limits.maxPageItems, limits);
      break;
    case "agent.listMcpServers":
      requireExactKeys(result, ["items", "nextCursor"], ["nextCursor"]);
      validateMcpServersResult(result, requestedLimit ?? limits.maxPageItems, limits);
      break;
    case "agent.listConversations":
      requireExactKeys(result, ["items", "nextCursor"], ["nextCursor"]);
      validateConversationsResult(result, requestedLimit ?? limits.maxPageItems, limits);
      break;
    case "agent.cancelConversation":
      requireExactKeys(result, ["disposition"]);
      if (result.disposition !== "accepted" && result.disposition !== "alreadyStopped") {
        throw new TypeError("cancel disposition is invalid");
      }
      break;
    case "agent.startConversation":
    case "agent.sendMessage":
      throw new TypeError("streaming methods require a generator result");
  }
  validatePlainJson(result);
  enforceEncodedLimit(result, limits.maxAgentResultBytes, "Agent result");
  return result;
}

/** Validates the branded business-error data before it enters a terminal error frame. */
export function validateAgentBusinessErrorData(
  value: unknown,
  limits: InitializeLimits,
): Record<string, unknown> {
  const data = requirePlainObject(value, "Agent business error data");
  requireExactKeys(data, ["kind", "retryable", "details"], ["details"]);
  if (
    ![
      "agentUnavailable",
      "authenticationRequired",
      "invalidAgentConfiguration",
      "installationNotFound",
      "conversationNotFound",
      "unsupportedAgentCapability",
      "invalidState",
      "permissionDenied",
      "cursorExpired",
      "agentProcessFailed",
      "providerFailure",
    ].includes(String(data.kind))
  ) {
    throw new TypeError("Agent business error kind is invalid");
  }
  if (typeof data.retryable !== "boolean") {
    throw new TypeError("Agent business error retryable must be boolean");
  }
  if (Object.hasOwn(data, "details")) {
    requirePlainObject(data.details, "Agent business error details");
  }
  validatePlainJson(data);
  enforceEncodedLimit(data, limits.maxAgentResultBytes, "Agent business error data");
  return data;
}

export function validatePlainJson(value: unknown, maximumDepth = 64): void {
  const ancestors = new Set<object>();
  visitJson(value, 1, maximumDepth, ancestors);
}

function visitJson(value: unknown, depth: number, maximumDepth: number, ancestors: Set<object>): void {
  if (depth > maximumDepth) {
    throw new TypeError("JSON value exceeds maximum depth");
  }
  if (value === null || typeof value === "string" || typeof value === "boolean") {
    return;
  }
  if (typeof value === "number") {
    if (!Number.isFinite(value)) {
      throw new TypeError("JSON number must be finite");
    }
    return;
  }
  if (typeof value !== "object") {
    throw new TypeError("value is not plain JSON");
  }
  if (ancestors.has(value)) {
    throw new TypeError("JSON value contains a cycle");
  }
  ancestors.add(value);
  if (Array.isArray(value)) {
    for (const item of value) {
      visitJson(item, depth + 1, maximumDepth, ancestors);
    }
  } else {
    const object = requirePlainObject(value, "JSON value");
    for (const item of Object.values(object)) {
      visitJson(item, depth + 1, maximumDepth, ancestors);
    }
  }
  ancestors.delete(value);
}

function validateInitializeLimits(value: unknown): InitializeLimits {
  const limits = requirePlainObject(value, "initialize limits");
  requireExactKeys(limits, [
    "maxFrameBytes",
    "maxPendingRequests",
    "maxAgentEventBytes",
    "maxAgentResultBytes",
    "maxAgentPromptBytes",
    "maxActiveTurns",
    "maxPageItems",
  ]);
  const maxima: Readonly<Record<string, number>> = {
    maxFrameBytes: 8 * 1024 * 1024,
    maxPendingRequests: 128,
    maxAgentEventBytes: 256 * 1024,
    maxAgentResultBytes: 1024 * 1024,
    maxAgentPromptBytes: 1024 * 1024,
    maxActiveTurns: 64,
    maxPageItems: 100,
  };
  for (const [field, maximum] of Object.entries(maxima)) {
    const current = limits[field];
    if (!Number.isSafeInteger(current) || (current as number) < 1 || (current as number) > maximum) {
      throw new TypeError(`${field} is outside its wire v1 cap`);
    }
  }
  if (limits.maxFrameBytes !== maxima.maxFrameBytes) {
    throw new TypeError("maxFrameBytes must equal the wire v1 constant");
  }
  return limits as unknown as InitializeLimits;
}

function validateScope(value: unknown): void {
  const scope = requirePlainObject(value, "Agent scope");
  switch (scope.type) {
    case "global":
      requireExactKeys(scope, ["type"]);
      break;
    case "project":
      requireExactKeys(scope, ["type", "projectHandle", "workingDirectory"]);
      requireOpaqueId(scope.projectHandle, "projectHandle");
      requireAbsoluteWindowsPath(scope.workingDirectory, "workingDirectory");
      break;
    case "worktree":
      requireExactKeys(scope, ["type", "projectHandle", "worktreeHandle", "workingDirectory"]);
      requireOpaqueId(scope.projectHandle, "projectHandle");
      requireOpaqueId(scope.worktreeHandle, "worktreeHandle");
      requireAbsoluteWindowsPath(scope.workingDirectory, "workingDirectory");
      break;
    default:
      throw new TypeError("Agent scope type is invalid");
  }
}

function validateUsage(value: unknown): void {
  const usage = requirePlainObject(value, "Agent usage");
  requireExactKeys(usage, ["inputTokens", "outputTokens", "costMicros"], [
    "inputTokens",
    "outputTokens",
    "costMicros",
  ]);
  if (Object.keys(usage).length === 0) {
    throw new TypeError("Agent usage must contain at least one counter");
  }
  for (const counter of Object.values(usage)) {
    if (!Number.isSafeInteger(counter) || (counter as number) < 0) {
      throw new TypeError("Agent usage counter must be a non-negative safe integer");
    }
  }
}

function requireHostRequestId(value: unknown): string {
  const id = requireBoundedString(value, "Host request id", 128);
  if (!/^h:(?:0|[1-9][0-9]*)$/u.test(id) || Number(id.slice(2)) > Number.MAX_SAFE_INTEGER) {
    throw new TypeError("Host request id is not canonical");
  }
  return id;
}

function requirePluginId(value: unknown): string {
  const id = requireBoundedString(value, "plugin id", 128);
  if (!/^[a-z0-9][a-z0-9._-]*\.[a-z0-9][a-z0-9._-]*$/u.test(id)) {
    throw new TypeError("plugin id is invalid");
  }
  return id;
}

function requireProviderId(value: unknown): string {
  const id = requireBoundedString(value, "provider id", 64);
  if (!/^[a-z0-9][a-z0-9._-]*$/u.test(id)) {
    throw new TypeError("provider id is invalid");
  }
  return id;
}

function requireOpaqueId(value: unknown, field: string): string {
  const id = requireBoundedString(value, field, 256);
  if (/[\x00-\x1f\x7f-\x9f/:\\]/u.test(id) || id.trim() !== id) {
    throw new TypeError(`${field} contains a forbidden character`);
  }
  return id;
}

function requireClientRequestId(value: unknown): string {
  const id = requireBoundedString(value, "clientRequestId", 36);
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/u.test(id)) {
    throw new TypeError("clientRequestId must be a canonical lower-case UUID");
  }
  return id;
}

function requirePrompt(value: unknown, maximumBytes: number): string {
  return requireBoundedString(value, "prompt", maximumBytes);
}

function requirePageLimit(value: unknown): number {
  if (!Number.isSafeInteger(value) || (value as number) < 1 || (value as number) > 100) {
    throw new TypeError("page limit must be in 1..=100");
  }
  return value as number;
}

function requireVersion(value: unknown, field: string): string {
  const version = requireBoundedString(value, field, 128);
  if (!/^(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/u.test(version)) {
    throw new TypeError(`${field} is not a canonical semantic version`);
  }
  return version;
}

function requireAbsoluteWindowsPath(value: unknown, field: string): string {
  const path = requireBoundedString(value, field, 32 * 1024);
  if (!/^(?:[A-Za-z]:[\\/]|[\\/]{2}[^\\/]+[\\/][^\\/]+[\\/])/u.test(path)) {
    throw new TypeError(`${field} must be an absolute Windows path`);
  }
  return path;
}

function requireDescendantPath(parent: string, child: string): void {
  const normalizedParent = parent.replaceAll("/", "\\").replace(/\\+$/u, "").toLowerCase();
  const normalizedChild = child.replaceAll("/", "\\").toLowerCase();
  const hasRelativeSegment = (path: string): boolean =>
    path.split("\\").some((component) => component === "." || component === "..");
  if (
    hasRelativeSegment(normalizedParent) ||
    hasRelativeSegment(normalizedChild) ||
    !normalizedChild.startsWith(`${normalizedParent}\\`)
  ) {
    throw new TypeError("entryPath must be a strict descendant of extensionPath");
  }
}

function requireDisplayString(value: unknown, field: string, maximumBytes: number): string {
  return requireBoundedString(value, field, maximumBytes);
}

function requireOptionalDisplayString(
  object: Record<string, unknown>,
  field: string,
  maximumBytes: number,
): void {
  if (Object.hasOwn(object, field)) {
    requireDisplayString(object[field], field, maximumBytes);
  }
}

function requireArray(value: unknown, field: string, maximumItems: number): unknown[] {
  if (!Array.isArray(value) || value.length > maximumItems) {
    throw new TypeError(`${field} exceeds its item cap`);
  }
  return value;
}

function enforceEncodedLimit(value: unknown, maximumBytes: number, field: string): void {
  const encoded = JSON.stringify(value);
  if (encoded === undefined || utf8Bytes(encoded) > maximumBytes) {
    throw new TypeError(`${field} exceeds its encoded byte cap`);
  }
}

function validateDiscoveryResult(result: Record<string, unknown>): void {
  const installations = requireArray(result.installations, "installations", 128);
  const diagnostics = requireArray(result.diagnostics, "diagnostics", 64);
  const installationIds = new Set<string>();
  for (const candidate of installations) {
    const installation = requirePlainObject(candidate, "Agent installation");
    requireExactKeys(
      installation,
      ["installationId", "displayName", "version", "locationDisplay", "availability"],
      ["version", "locationDisplay"],
    );
    const id = requireOpaqueId(installation.installationId, "installationId");
    requireUnique(installationIds, id, "installationId");
    requireDisplayString(installation.displayName, "installation displayName", 512);
    requireOptionalDisplayString(installation, "version", 512);
    requireOptionalDisplayString(installation, "locationDisplay", 4096);
    const availability = requirePlainObject(installation.availability, "Agent availability");
    if (availability.type === "available") {
      requireExactKeys(availability, ["type"]);
    } else if (availability.type === "unavailable") {
      requireExactKeys(availability, ["type", "reason"]);
      requireDisplayString(availability.reason, "availability reason", 4096);
    } else {
      throw new TypeError("Agent availability type is invalid");
    }
  }

  let hasNotFound = false;
  for (const candidate of diagnostics) {
    const diagnostic = requirePlainObject(candidate, "Agent discovery diagnostic");
    requireExactKeys(diagnostic, ["kind", "message"]);
    if (!["notFound", "permissionDenied", "probeFailed"].includes(String(diagnostic.kind))) {
      throw new TypeError("Agent discovery diagnostic kind is invalid");
    }
    hasNotFound ||= diagnostic.kind === "notFound";
    requireDisplayString(diagnostic.message, "diagnostic message", 4096);
  }
  if (installations.length === 0 && !hasNotFound) {
    throw new TypeError("empty discovery requires a notFound diagnostic");
  }
}

function validateConfigurationResult(result: Record<string, unknown>): void {
  const items = requireArray(result.items, "configuration items", 256);
  const keys = new Set<string>();
  for (const candidate of items) {
    const item = requirePlainObject(candidate, "Agent configuration item");
    requireExactKeys(item, ["key", "displayName", "source", "value"]);
    const key = requireConfigurationKey(item.key);
    requireUnique(keys, key, "configuration key");
    requireDisplayString(item.displayName, "configuration displayName", 512);
    validateResourceSource(item.source);
    validateConfigurationValue(item.value);
  }
}

function validateSkillsResult(
  result: Record<string, unknown>,
  requestedLimit: number,
  limits: InitializeLimits,
): void {
  const items = requirePageItems(result, requestedLimit, limits);
  const ids = new Set<string>();
  for (const candidate of items) {
    const item = requirePlainObject(candidate, "Agent skill summary");
    requireExactKeys(item, ["id", "displayName", "description", "source"], ["description"]);
    requireUnique(ids, requireOpaqueId(item.id, "skill id"), "skill id");
    requireDisplayString(item.displayName, "skill displayName", 512);
    requireOptionalDisplayString(item, "description", 4096);
    validateResourceSource(item.source);
  }
}

function validateMcpServersResult(
  result: Record<string, unknown>,
  requestedLimit: number,
  limits: InitializeLimits,
): void {
  const items = requirePageItems(result, requestedLimit, limits);
  const ids = new Set<string>();
  for (const candidate of items) {
    const item = requirePlainObject(candidate, "Agent MCP server summary");
    requireExactKeys(item, ["id", "displayName", "transport", "enabled", "source"]);
    requireUnique(ids, requireOpaqueId(item.id, "MCP server id"), "MCP server id");
    requireDisplayString(item.displayName, "MCP server displayName", 512);
    if (!["stdio", "http", "sse", "unknown"].includes(String(item.transport))) {
      throw new TypeError("MCP transport is invalid");
    }
    if (typeof item.enabled !== "boolean") {
      throw new TypeError("MCP enabled must be boolean");
    }
    validateResourceSource(item.source);
  }
}

function validateConversationsResult(
  result: Record<string, unknown>,
  requestedLimit: number,
  limits: InitializeLimits,
): void {
  const items = requirePageItems(result, requestedLimit, limits);
  const ids = new Set<string>();
  for (const candidate of items) {
    const item = requirePlainObject(candidate, "Agent conversation summary");
    requireExactKeys(item, ["conversationId", "title", "updatedAt"], ["title", "updatedAt"]);
    requireUnique(
      ids,
      requireOpaqueId(item.conversationId, "conversationId"),
      "conversationId",
    );
    requireOptionalDisplayString(item, "title", 4096);
    if (Object.hasOwn(item, "updatedAt")) {
      const timestamp = requireBoundedString(item.updatedAt, "updatedAt", 64);
      if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/u.test(timestamp)) {
        throw new TypeError("updatedAt must be RFC3339");
      }
    }
  }
}

function requirePageItems(
  result: Record<string, unknown>,
  requestedLimit: number,
  limits: InitializeLimits,
): unknown[] {
  const items = requireArray(
    result.items,
    "page items",
    Math.min(requestedLimit, limits.maxPageItems),
  );
  if (Object.hasOwn(result, "nextCursor")) {
    requireOpaqueId(result.nextCursor, "nextCursor");
  }
  return items;
}

function validateResourceSource(value: unknown): void {
  const source = requirePlainObject(value, "Agent resource source");
  if (["user", "project", "worktree", "builtIn"].includes(String(source.type))) {
    requireExactKeys(source, ["type"]);
  } else if (source.type === "unknown") {
    requireExactKeys(source, ["type", "display"], ["display"]);
    requireOptionalDisplayString(source, "display", 4096);
  } else {
    throw new TypeError("Agent resource source is invalid");
  }
}

function validateConfigurationValue(value: unknown): void {
  const configuration = requirePlainObject(value, "Agent configuration value");
  switch (configuration.type) {
    case "unset":
    case "redacted":
      requireExactKeys(configuration, ["type"]);
      break;
    case "boolean":
      requireExactKeys(configuration, ["type", "value"]);
      if (typeof configuration.value !== "boolean") {
        throw new TypeError("configuration boolean value is invalid");
      }
      break;
    case "number":
      requireExactKeys(configuration, ["type", "value"]);
      if (typeof configuration.value !== "number" || !Number.isFinite(configuration.value)) {
        throw new TypeError("configuration number value is invalid");
      }
      break;
    case "string":
      requireExactKeys(configuration, ["type", "value"]);
      requireDisplayString(configuration.value, "configuration string", 4096);
      break;
    case "stringList": {
      requireExactKeys(configuration, ["type", "value"]);
      const values = requireArray(configuration.value, "configuration string list", 128);
      for (const item of values) {
        requireDisplayString(item, "configuration string-list item", 4096);
      }
      break;
    }
    default:
      throw new TypeError("Agent configuration value type is invalid");
  }
}

function requireConfigurationKey(value: unknown): string {
  const key = requireBoundedString(value, "configuration key", 512);
  if (!/^[0-9A-Za-z][0-9A-Za-z._-]*$/u.test(key)) {
    throw new TypeError("configuration key is invalid");
  }
  return key;
}

function requireUnique(values: Set<string>, value: string, field: string): void {
  if (values.has(value)) {
    throw new TypeError(`${field} must be unique`);
  }
  values.add(value);
}
