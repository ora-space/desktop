import { FrameType, type Frame } from "../transport/frame.js";
import { parseStrictJson } from "../json/strict.js";

export interface RpcRequest {
  readonly type: "request";
  readonly id: string;
  readonly method: string;
  readonly params?: Record<string, unknown>;
}

export interface RpcNotification {
  readonly type: "notification";
  readonly method: string;
  readonly params?: Record<string, unknown>;
}

export type InboundEnvelope = RpcRequest | RpcNotification;

/** Parses Host-to-bootstrap JSON frames; envelope kind comes from JSON shape, not wire type. */
export function parseInboundEnvelope(frame: Frame, maximumDepth = 64): InboundEnvelope {
  if (frame.type !== FrameType.Json) {
    throw new SyntaxError("bootstrap only accepts JSON payload frames");
  }
  const value = parseStrictJson(frame.payload, maximumDepth);
  const object = requirePlainObject(value, "JSON-RPC envelope");
  if (object.jsonrpc !== "2.0") {
    throw new SyntaxError("JSON-RPC version must equal 2.0");
  }
  if (typeof object.method === "string") {
    if (Object.hasOwn(object, "id")) {
      requireExactKeys(object, ["jsonrpc", "id", "method", "params"], ["params"]);
      const id = requireBoundedString(object.id, "request id", 128);
      const method = requireBoundedString(object.method, "request method", 256);
      const params = parseOptionalParams(object);
      return params === undefined
        ? { type: "request", id, method }
        : { type: "request", id, method, params };
    }
    requireExactKeys(object, ["jsonrpc", "method", "params"], ["params"]);
    const method = requireBoundedString(object.method, "notification method", 256);
    const params = parseOptionalParams(object);
    return params === undefined
      ? { type: "notification", method }
      : { type: "notification", method, params };
  }
  throw new SyntaxError("Host cannot send a Response frame to the bootstrap");
}

/** Encodes one strict success response without allowing undefined or non-JSON data. */
export function encodeSuccess(id: string, result: unknown): Uint8Array {
  return encodeJson({ jsonrpc: "2.0", id, result });
}

/** Encodes one strict error response with the optional data field omitted rather than null. */
export function encodeError(
  id: string,
  code: number,
  message: string,
  data?: Record<string, unknown>,
): Uint8Array {
  const error = data === undefined ? { code, message } : { code, message, data };
  return encodeJson({ jsonrpc: "2.0", id, error });
}

/** Encodes one stream notification tied to a Host request id and exact sequence number. */
export function encodeStream(id: string, seq: number, value: unknown): Uint8Array {
  return encodeJson({ jsonrpc: "2.0", method: "$/stream", params: { id, seq, value } });
}

export function requirePlainObject(value: unknown, field: string): Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError(`${field} must be an object`);
  }
  const prototype = Object.getPrototypeOf(value) as object | null;
  if (prototype !== Object.prototype && prototype !== null) {
    throw new TypeError(`${field} must be a plain object`);
  }
  return value as Record<string, unknown>;
}

export function requireExactKeys(
  value: Record<string, unknown>,
  allowed: readonly string[],
  optional: readonly string[] = [],
): void {
  const allowedSet = new Set(allowed);
  const optionalSet = new Set(optional);
  for (const key of Object.keys(value)) {
    if (!allowedSet.has(key)) {
      throw new TypeError(`unknown field '${key}'`);
    }
  }
  for (const key of allowed) {
    if (!optionalSet.has(key) && !Object.hasOwn(value, key)) {
      throw new TypeError(`missing field '${key}'`);
    }
    if (Object.hasOwn(value, key) && value[key] === null) {
      throw new TypeError(`field '${key}' cannot be null`);
    }
  }
}

export function requireBoundedString(value: unknown, field: string, maximumBytes: number): string {
  if (typeof value !== "string" || value.length === 0 || utf8Bytes(value) > maximumBytes || value.includes("\0")) {
    throw new TypeError(`${field} must be a non-empty bounded string without NUL`);
  }
  return value;
}

export function utf8Bytes(value: string): number {
  return new TextEncoder().encode(value).byteLength;
}

function parseOptionalParams(object: Record<string, unknown>): Record<string, unknown> | undefined {
  if (!Object.hasOwn(object, "params")) {
    return undefined;
  }
  return requirePlainObject(object.params, "JSON-RPC params");
}

function encodeJson(value: unknown): Uint8Array {
  return new TextEncoder().encode(JSON.stringify(value));
}
