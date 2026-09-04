import type { PluginInvocationContext } from "./context.ts";
import type { Plugin } from "./plugin.ts";
import type { JsonValue } from "./protocol/index.ts";
import { decodeBase64 } from "./storage.ts";

const TRACE_LIST = "ora/trace/list";
const TRACE_STAT = "ora/trace/stat";
const TRACE_READ = "ora/trace/read";

/** Default and maximum chunk size accepted by Ora's v1 trace data plane. */
export const DEFAULT_TRACE_CHUNK_BYTES = 1024 * 1024;
export const MAX_TRACE_CHUNK_BYTES = 4 * 1024 * 1024;

/** Metadata for one trace file authorized by the invocation context. */
export interface TraceResource {
  traceId: string;
  providerId: string;
  format: string;
  sizeBytes: number;
  modifiedAtMs: number;
  /** Changes when the underlying file is replaced or truncated. */
  cursor: string;
  /** A host-sanitized session title suitable for trace selection UI. */
  label: string;
  /** The trace for the session from which this dashboard was opened. */
  isCurrent: boolean;
}

/** One bounded raw byte range read by the host. */
export interface TraceChunk {
  bytes: Uint8Array;
  offset: number;
  nextOffset: number;
  eof: boolean;
  cursor: string;
}

/** Session-scoped trace access. The context is included automatically on every request. */
export interface TraceClient {
  list(): Promise<TraceResource[]>;
  stat(traceId: string): Promise<TraceResource>;
  read(
    traceId: string,
    offset?: number,
    maxBytes?: number,
    cursor?: string,
  ): Promise<TraceChunk>;
}

/** Builds a trace client whose authority is fixed to one host-issued invocation context. */
export function createTraceClient(
  plugin: Plugin,
  context: PluginInvocationContext,
): TraceClient {
  return {
    async list() {
      const result = await plugin.request(TRACE_LIST, {
        context_id: context.id,
      });
      const object = asRecord(result);
      if (object === undefined || !Array.isArray(object.traces)) {
        throw new Error(`${TRACE_LIST} returned an invalid result`);
      }
      return object.traces.map((entry) => parseResource(TRACE_LIST, entry));
    },
    async stat(traceId) {
      const result = await plugin.request(TRACE_STAT, {
        context_id: context.id,
        trace_id: traceId,
      });
      return parseResource(TRACE_STAT, result);
    },
    async read(
      traceId,
      offset = 0,
      maxBytes = DEFAULT_TRACE_CHUNK_BYTES,
      cursor,
    ) {
      if (!Number.isSafeInteger(offset) || offset < 0) {
        throw new Error(
          "Trace read offset must be a non-negative safe integer",
        );
      }
      if (
        !Number.isSafeInteger(maxBytes) || maxBytes < 1 ||
        maxBytes > MAX_TRACE_CHUNK_BYTES
      ) {
        throw new Error(
          `Trace read maxBytes must be between 1 and ${MAX_TRACE_CHUNK_BYTES}`,
        );
      }
      const result = await plugin.request(TRACE_READ, {
        context_id: context.id,
        trace_id: traceId,
        offset,
        max_bytes: maxBytes,
        cursor: cursor ?? null,
      });
      const object = asRecord(result);
      if (
        object === undefined || typeof object.bytes_base64 !== "string" ||
        typeof object.offset !== "number" ||
        typeof object.next_offset !== "number" ||
        typeof object.eof !== "boolean" || typeof object.cursor !== "string"
      ) {
        throw new Error(`${TRACE_READ} returned an invalid result`);
      }
      return {
        bytes: decodeBase64(object.bytes_base64),
        offset: object.offset,
        nextOffset: object.next_offset,
        eof: object.eof,
        cursor: object.cursor,
      };
    },
  };
}

function parseResource(method: string, value: JsonValue): TraceResource {
  const object = asRecord(value);
  if (
    object === undefined || typeof object.trace_id !== "string" ||
    typeof object.provider_id !== "string" || typeof object.format !== "string" ||
    typeof object.size_bytes !== "number" ||
    typeof object.modified_at_ms !== "number" || typeof object.cursor !== "string" ||
    typeof object.label !== "string" || typeof object.is_current !== "boolean"
  ) {
    throw new Error(`${method} returned an invalid trace resource`);
  }
  return {
    traceId: object.trace_id,
    providerId: object.provider_id,
    format: object.format,
    sizeBytes: object.size_bytes,
    modifiedAtMs: object.modified_at_ms,
    cursor: object.cursor,
    label: object.label,
    isCurrent: object.is_current,
  };
}

function asRecord(value: JsonValue): Record<string, JsonValue> | undefined {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, JsonValue>
    : undefined;
}
