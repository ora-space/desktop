import { PluginMethodError } from "./plugin.ts";
import type { JsonValue } from "./protocol.ts";

/** JSON-RPC method that must be registered together with `mcpConfiguration`. */
export const AGENT_CONFIGURE_WORKSPACE = "agent/configureWorkspace";

/** Transport kinds protocol v1 may materialize. */
export type McpTransportKind = "http" | "stdio";

/** Coordination mode protocol v1 may negotiate. */
export type McpCoordinationMode = "wait_for_idle_and_restart";

/** Wire capability published on `ora/register` when MCP materialization is supported. */
export interface McpConfigurationCapabilityDeclaration {
  protocolVersion: 1;
  transports: readonly McpTransportKind[];
  coordination: McpCoordinationMode;
}

/** One Streamable HTTP MCP in a full snapshot. Header values stay on the wire, never in errors. */
export interface ResolvedHttpMcpTransport {
  kind: "http";
  url: string;
  headers: Readonly<Record<string, string>>;
}

/** One stdio MCP in a full snapshot. Environment values stay on the wire, never in errors. */
export interface ResolvedStdioMcpTransport {
  kind: "stdio";
  executable: string;
  args: readonly string[];
  env: Readonly<Record<string, string>>;
  workingDirectory: string;
}

export type ResolvedMcpTransport =
  | ResolvedHttpMcpTransport
  | ResolvedStdioMcpTransport;

/** One target-independent Resolved MCP included in a complete snapshot. */
export interface SnapshotResolvedMcp {
  canonicalIdentity: string;
  managedIdentity: string;
  packageVersion: string;
  sourceRevisionId: string;
  transport: ResolvedMcpTransport;
}

/** Complete MCP Configuration Snapshot the Host sends to `agent/configureWorkspace`. */
export interface McpConfigurationSnapshotRequest {
  protocolVersion: 1;
  operationId: string;
  agentTargetId: string;
  workspaceRoot: string;
  generation: number;
  resolvedMcps: readonly SnapshotResolvedMcp[];
}

/** One managed entry the plugin applied for the current snapshot. */
export interface McpEntryReceipt {
  managedIdentity: string;
  nativeKey: string;
  entryFingerprint: string;
  sourceRevisionId: string;
}

/** Success receipt returned after the plugin durably applied a snapshot. */
export interface McpConfigurationReceipt {
  appliedGeneration: number;
  documentLocator: string;
  documentFingerprint: string;
  entries: readonly McpEntryReceipt[];
}

/** Registers capability and handler together so authors cannot declare only one side. */
export interface AgentMcpConfigurationDefinition
  extends McpConfigurationCapabilityDeclaration {
  configureWorkspace(
    request: McpConfigurationSnapshotRequest,
  ): McpConfigurationReceipt | Promise<McpConfigurationReceipt>;
}

const REQUEST_FIELDS = new Set([
  "protocolVersion",
  "operationId",
  "agentTargetId",
  "workspaceRoot",
  "generation",
  "resolvedMcps",
]);
const RESOLVED_FIELDS = new Set([
  "canonicalIdentity",
  "managedIdentity",
  "packageVersion",
  "sourceRevisionId",
  "transport",
]);
const HTTP_FIELDS = new Set(["kind", "url", "headers"]);
const STDIO_FIELDS = new Set([
  "kind",
  "executable",
  "args",
  "env",
  "workingDirectory",
]);

/** Rejects a capability that would serialize into an invalid Host declaration. */
export function validateMcpConfigurationCapability(
  capability: McpConfigurationCapabilityDeclaration,
): void {
  if (capability.protocolVersion !== 1) {
    throw new Error("mcpConfiguration protocolVersion must be 1");
  }
  if (capability.coordination !== "wait_for_idle_and_restart") {
    throw new Error("mcpConfiguration coordination is not supported");
  }
  if (capability.transports.length === 0) {
    throw new Error("mcpConfiguration transports must not be empty");
  }
  const seen = new Set<string>();
  for (const transport of capability.transports) {
    if (transport !== "http" && transport !== "stdio") {
      throw new Error("mcpConfiguration transports contains an unknown kind");
    }
    if (seen.has(transport)) {
      throw new Error("mcpConfiguration transports must be unique");
    }
    seen.add(transport);
  }
}

/** Parses a Host snapshot before plugin-owned rendering so malformed requests never reach it. */
export function parseMcpConfigurationSnapshotRequest(
  input: JsonValue,
): McpConfigurationSnapshotRequest {
  if (!isRecord(input)) {
    throw invalidSnapshot();
  }
  rejectUnknownFields(input, REQUEST_FIELDS);
  if (
    input.protocolVersion !== 1 ||
    typeof input.operationId !== "string" ||
    input.operationId.length === 0 ||
    typeof input.agentTargetId !== "string" ||
    input.agentTargetId.length === 0 ||
    typeof input.workspaceRoot !== "string" ||
    !isAbsoluteWorkspaceRoot(input.workspaceRoot) ||
    typeof input.generation !== "number" ||
    !Number.isSafeInteger(input.generation) ||
    input.generation < 0 ||
    !Array.isArray(input.resolvedMcps)
  ) {
    throw invalidSnapshot();
  }
  return {
    protocolVersion: 1,
    operationId: input.operationId,
    agentTargetId: input.agentTargetId,
    workspaceRoot: input.workspaceRoot,
    generation: input.generation,
    resolvedMcps: input.resolvedMcps.map(parseResolvedMcp),
  };
}

/** Serializes a receipt with the closed field set Host validation expects. */
export function serializeMcpConfigurationReceipt(
  receipt: McpConfigurationReceipt,
): JsonValue {
  return {
    appliedGeneration: receipt.appliedGeneration,
    documentLocator: receipt.documentLocator,
    documentFingerprint: normalizeFingerprint(receipt.documentFingerprint) ??
      receipt.documentFingerprint,
    entries: receipt.entries.map((entry) => ({
      managedIdentity: entry.managedIdentity,
      nativeKey: entry.nativeKey,
      entryFingerprint: normalizeFingerprint(entry.entryFingerprint) ??
        entry.entryFingerprint,
      sourceRevisionId: entry.sourceRevisionId,
    })),
  };
}

/** Wire parse of optional `mcpConfiguration` without failing the baseline agent contract. */
export type ParsedMcpConfigurationRegistration =
  | { status: "absent" }
  | {
    status: "invalid";
    code: "mcp_capability_invalid" | "mcp_capability_version_unsupported";
  }
  | { status: "declared"; capability: McpConfigurationCapabilityDeclaration };

/** Host pairing of capability and `agent/configureWorkspace` on a registration payload. */
export type NegotiatedMcpConfiguration =
  | { status: "unsupported" }
  | {
    status: "disabled";
    code: "mcp_capability_invalid" | "mcp_capability_version_unsupported";
  }
  | { status: "enabled"; capability: McpConfigurationCapabilityDeclaration };

const CAPABILITY_FIELDS = new Set([
  "protocolVersion",
  "transports",
  "coordination",
]);
const RECEIPT_FIELDS = new Set([
  "appliedGeneration",
  "documentLocator",
  "documentFingerprint",
  "entries",
]);
const ENTRY_RECEIPT_FIELDS = new Set([
  "managedIdentity",
  "nativeKey",
  "entryFingerprint",
  "sourceRevisionId",
]);
const FINGERPRINT = /^sha256:([a-fA-F0-9]{64})$/;

/** Parses registration JSON the same way Host handshake classifies the optional capability. */
export function parseMcpConfigurationRegistration(
  params: JsonValue,
): ParsedMcpConfigurationRegistration {
  if (!isRecord(params) || !("mcpConfiguration" in params)) {
    return { status: "absent" };
  }
  return parseMcpConfigurationValue(params.mcpConfiguration);
}

/**
 * Pairs capability with handler so shared fixtures can assert Host negotiation without Deno.
 *
 * defineAgent already refuses a one-sided high-level API; this classifies raw registration JSON.
 */
export function negotiateMcpConfiguration(
  params: JsonValue,
): NegotiatedMcpConfiguration {
  const methods = isRecord(params) && Array.isArray(params.methods)
    ? params.methods.filter((entry): entry is string =>
      typeof entry === "string"
    )
    : [];
  const hasHandler = methods.includes(AGENT_CONFIGURE_WORKSPACE);
  const registration = parseMcpConfigurationRegistration(params);
  if (registration.status === "absent") {
    return hasHandler
      ? { status: "disabled", code: "mcp_capability_invalid" }
      : { status: "unsupported" };
  }
  if (registration.status === "invalid") {
    return { status: "disabled", code: registration.code };
  }
  return hasHandler
    ? { status: "enabled", capability: registration.capability }
    : { status: "disabled", code: "mcp_capability_invalid" };
}

/** Stable Host receipt rejection codes shared with Rust compatibility fixtures. */
export type ReceiptValidationCode =
  | "invalid_structure"
  | "generation_mismatch"
  | "locator_out_of_bounds"
  | "missing_managed_identity"
  | "duplicate_managed_identity"
  | "duplicate_native_key"
  | "extra_managed_identity"
  | "source_revision_mismatch"
  | "illegal_fingerprint";

export type ParsedMcpConfigurationReceipt =
  | { ok: true; receipt: McpConfigurationReceipt }
  | { ok: false; code: ReceiptValidationCode };

/** Desired identities a success receipt must cover exactly. */
export interface ExpectedReceiptCoverage {
  generation: number;
  desired: readonly { managedIdentity: string; sourceRevisionId: string }[];
}

/** Parses a plugin receipt using the closed field set Host validation requires. */
export function parseMcpConfigurationReceipt(
  input: JsonValue,
): ParsedMcpConfigurationReceipt {
  if (!isRecord(input)) {
    return { ok: false, code: "invalid_structure" };
  }
  try {
    rejectUnknownFields(input, RECEIPT_FIELDS);
  } catch {
    return { ok: false, code: "invalid_structure" };
  }
  if (
    typeof input.appliedGeneration !== "number" ||
    !Number.isSafeInteger(input.appliedGeneration) ||
    input.appliedGeneration < 0 ||
    typeof input.documentLocator !== "string" ||
    typeof input.documentFingerprint !== "string" ||
    !Array.isArray(input.entries)
  ) {
    return { ok: false, code: "invalid_structure" };
  }
  if (!isWorkspaceRelativeLocator(input.documentLocator)) {
    return { ok: false, code: "locator_out_of_bounds" };
  }
  const documentFingerprint = normalizeFingerprint(input.documentFingerprint);
  if (documentFingerprint === undefined) {
    return { ok: false, code: "illegal_fingerprint" };
  }
  const entries: McpEntryReceipt[] = [];
  for (const entry of input.entries) {
    if (!isRecord(entry)) {
      return { ok: false, code: "invalid_structure" };
    }
    try {
      rejectUnknownFields(entry, ENTRY_RECEIPT_FIELDS);
    } catch {
      return { ok: false, code: "invalid_structure" };
    }
    if (
      typeof entry.managedIdentity !== "string" ||
      entry.managedIdentity.length === 0 ||
      typeof entry.nativeKey !== "string" ||
      entry.nativeKey.length === 0 ||
      typeof entry.entryFingerprint !== "string" ||
      typeof entry.sourceRevisionId !== "string"
    ) {
      return { ok: false, code: "invalid_structure" };
    }
    const entryFingerprint = normalizeFingerprint(entry.entryFingerprint);
    if (entryFingerprint === undefined) {
      return { ok: false, code: "illegal_fingerprint" };
    }
    entries.push({
      managedIdentity: entry.managedIdentity,
      nativeKey: entry.nativeKey,
      entryFingerprint,
      sourceRevisionId: entry.sourceRevisionId,
    });
  }
  return {
    ok: true,
    receipt: {
      appliedGeneration: input.appliedGeneration,
      documentLocator: input.documentLocator,
      documentFingerprint,
      entries,
    },
  };
}

/** Rejects receipts that would let an incomplete plugin result be marked Ready. */
export function validateMcpConfigurationReceiptCoverage(
  receipt: McpConfigurationReceipt,
  expected: ExpectedReceiptCoverage,
): ReceiptValidationCode | undefined {
  if (receipt.appliedGeneration !== expected.generation) {
    return "generation_mismatch";
  }
  const expectedById = new Map(
    expected.desired.map((
      entry,
    ) => [entry.managedIdentity, entry.sourceRevisionId]),
  );
  const seenIdentities = new Set<string>();
  const seenNativeKeys = new Set<string>();
  for (const entry of receipt.entries) {
    if (seenIdentities.has(entry.managedIdentity)) {
      return "duplicate_managed_identity";
    }
    seenIdentities.add(entry.managedIdentity);
    if (seenNativeKeys.has(entry.nativeKey)) {
      return "duplicate_native_key";
    }
    seenNativeKeys.add(entry.nativeKey);
    const expectedRevision = expectedById.get(entry.managedIdentity);
    if (expectedRevision === undefined) {
      return "extra_managed_identity";
    }
    if (entry.sourceRevisionId !== expectedRevision) {
      return "source_revision_mismatch";
    }
  }
  if (seenIdentities.size !== expected.desired.length) {
    return "missing_managed_identity";
  }
  return undefined;
}

function parseMcpConfigurationValue(
  value: unknown,
): ParsedMcpConfigurationRegistration {
  if (!isRecord(value)) {
    return { status: "invalid", code: "mcp_capability_invalid" };
  }
  for (const key of Object.keys(value)) {
    if (!CAPABILITY_FIELDS.has(key)) {
      return { status: "invalid", code: "mcp_capability_invalid" };
    }
  }
  if (
    value.protocolVersion === 0 || !Number.isSafeInteger(value.protocolVersion)
  ) {
    return { status: "invalid", code: "mcp_capability_invalid" };
  }
  if (
    typeof value.protocolVersion === "number" && value.protocolVersion !== 1
  ) {
    return { status: "invalid", code: "mcp_capability_version_unsupported" };
  }
  if (
    value.protocolVersion !== 1 ||
    value.coordination !== "wait_for_idle_and_restart" ||
    !Array.isArray(value.transports)
  ) {
    return { status: "invalid", code: "mcp_capability_invalid" };
  }
  try {
    const capability: McpConfigurationCapabilityDeclaration = {
      protocolVersion: 1,
      transports: [...value.transports] as McpTransportKind[],
      coordination: "wait_for_idle_and_restart",
    };
    validateMcpConfigurationCapability(capability);
    return { status: "declared", capability };
  } catch {
    return { status: "invalid", code: "mcp_capability_invalid" };
  }
}

function isWorkspaceRelativeLocator(value: string): boolean {
  if (
    value.length === 0 || value === "." || value.startsWith("/") ||
    value.startsWith("\\")
  ) {
    return false;
  }
  if (/^[A-Za-z]:/.test(value)) {
    return false;
  }
  return value.split(/[/\\]/).every((part) =>
    part.length > 0 && part !== ".." && part !== "."
  );
}

function parseResolvedMcp(value: unknown): SnapshotResolvedMcp {
  if (!isRecord(value)) {
    throw invalidSnapshot();
  }
  rejectUnknownFields(value, RESOLVED_FIELDS);
  if (
    typeof value.canonicalIdentity !== "string" ||
    value.canonicalIdentity.length === 0 ||
    typeof value.managedIdentity !== "string" ||
    value.managedIdentity.length === 0 ||
    typeof value.packageVersion !== "string" ||
    value.packageVersion.length === 0 ||
    typeof value.sourceRevisionId !== "string" ||
    value.sourceRevisionId.length === 0
  ) {
    throw invalidSnapshot();
  }
  return {
    canonicalIdentity: value.canonicalIdentity,
    managedIdentity: value.managedIdentity,
    packageVersion: value.packageVersion,
    sourceRevisionId: value.sourceRevisionId,
    transport: parseTransport(value.transport),
  };
}

function parseTransport(value: unknown): ResolvedMcpTransport {
  if (!isRecord(value) || typeof value.kind !== "string") {
    throw invalidSnapshot();
  }
  if (value.kind === "http") {
    rejectUnknownFields(value, HTTP_FIELDS);
    if (
      typeof value.url !== "string" ||
      !isAbsoluteHttpUrl(value.url) ||
      !isRecord(value.headers) ||
      !stringMap(value.headers)
    ) {
      throw invalidSnapshot();
    }
    return {
      kind: "http",
      url: value.url,
      headers: { ...value.headers },
    };
  }
  if (value.kind === "stdio") {
    rejectUnknownFields(value, STDIO_FIELDS);
    if (
      typeof value.executable !== "string" ||
      value.executable.length === 0 ||
      !Array.isArray(value.args) ||
      !value.args.every((entry) => typeof entry === "string") ||
      !isRecord(value.env) ||
      !stringMap(value.env) ||
      typeof value.workingDirectory !== "string" ||
      value.workingDirectory.length === 0
    ) {
      throw invalidSnapshot();
    }
    return {
      kind: "stdio",
      executable: value.executable,
      args: [...value.args],
      env: { ...value.env },
      workingDirectory: value.workingDirectory,
    };
  }
  throw invalidSnapshot();
}

function rejectUnknownFields(
  value: Record<string, unknown>,
  allowed: Set<string>,
): void {
  for (const key of Object.keys(value)) {
    if (!allowed.has(key)) {
      throw invalidSnapshot();
    }
  }
}

function stringMap(
  value: Record<string, unknown>,
): value is Record<string, string> {
  return Object.entries(value).every(([key, entry]) =>
    key.length > 0 && typeof entry === "string"
  );
}

/** Host snapshots carry absolute HTTP(S) URLs; relative and non-HTTP schemes are rejected. */
function isAbsoluteHttpUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return (url.protocol === "http:" || url.protocol === "https:") &&
      url.hostname.length > 0;
  } catch {
    return false;
  }
}

/** Accepts SHA-256 hex in either case and stores the Host-canonical lowercase form. */
function normalizeFingerprint(value: string): string | undefined {
  const match = FINGERPRINT.exec(value);
  if (match === null) {
    return undefined;
  }
  return `sha256:${match[1].toLowerCase()}`;
}

function isAbsoluteWorkspaceRoot(value: string): boolean {
  return value.startsWith("/") || /^[A-Za-z]:[\\/]/.test(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Messages stay generic so header, environment, and document values cannot leak into JSON-RPC. */
function invalidSnapshot(): PluginMethodError {
  return new PluginMethodError(
    -32602,
    "agent/configureWorkspace requires a protocol v1 snapshot",
  );
}
