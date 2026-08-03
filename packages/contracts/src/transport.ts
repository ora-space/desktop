import { z } from "zod";
import type { ContractError } from "./error.js";
import { contractErrorSchema, publicErrorSchema } from "./error.schema.js";

export type HttpMethod = "GET" | "POST" | "PUT" | "DELETE";

export type ContractTransportRequest = {
  operationName: string;
  request: unknown;
  method: HttpMethod;
  path: string;
  body: unknown | undefined;
  headers: Record<string, string>;
};

export type ContractCallOptions = {
  readonly signal?: AbortSignal;
};

export interface ContractTransport {
  send<TResponse>(request: ContractTransportRequest, options?: ContractCallOptions): Promise<TResponse>;
  stream<TEvent>(request: ContractTransportRequest, options?: ContractCallOptions): AsyncIterable<TEvent>;
}

export type ContractStreamFrame<TEvent> =
  | { type: "data"; data: TEvent }
  | { type: "error"; error: unknown }
  | { type: "end" };

export const localTransportErrorKinds = [
  "network_failure",
  "tauri_invoke_failure",
  "malformed_response",
  "malformed_stream_frame",
  "stream_interrupted",
  "stream_frame_too_large",
  "stream_queue_overflow",
  "stream_already_consumed",
  "unsupported_operation",
  "cancelled",
] as const;

export type LocalTransportErrorKind = (typeof localTransportErrorKinds)[number];

/** Carries one runtime-validated error produced by an Ora adapter. */
export class RemoteContractError extends Error {
  readonly payload: ContractError;
  readonly status: number | null;
  readonly responseBody: unknown;

  constructor(payload: ContractError, status: number | null, responseBody: unknown) {
    super(`Remote Ora request failed with ${payload.code} (${payload.requestId})`);
    this.name = "RemoteContractError";
    this.payload = payload;
    this.status = status;
    this.responseBody = responseBody;
  }

  get code(): ContractError["code"] {
    return this.payload.code;
  }

  get requestId(): string {
    return this.payload.requestId;
  }
}

/** Preserves correlation when a newer backend returns a code this frontend does not know. */
export class UnknownRemoteError extends Error {
  readonly rawCode: string;
  readonly requestId: string;
  readonly status: number | null;
  readonly responseBody: unknown;

  constructor(rawCode: string, requestId: string, status: number | null, responseBody: unknown) {
    super(`Remote Ora request returned unknown code ${rawCode} (${requestId})`);
    this.name = "UnknownRemoteError";
    this.rawCode = rawCode;
    this.requestId = requestId;
    this.status = status;
    this.responseBody = responseBody;
  }
}

/** Represents a finite failure of the local Web or Tauri transport itself. */
export class LocalTransportError extends Error {
  readonly kind: LocalTransportErrorKind;
  readonly causeValue: unknown;

  constructor(kind: LocalTransportErrorKind, technicalMessage: string, causeValue: unknown = null) {
    super(technicalMessage);
    this.name = "LocalTransportError";
    this.kind = kind;
    this.causeValue = causeValue;
  }
}

const remoteBaseSchema = z.object({
  code: z.string(),
  params: z.unknown(),
  requestId: z.uuid(),
});
const knownCodes: ReadonlySet<string> = new Set(
  publicErrorSchema.options.map((option) => option.shape.code.value),
);

/** Validates the shared remote payload and distinguishes unknown codes from malformed responses. */
export function decodeRemoteError(
  value: unknown,
  status: number | null,
  responseBody: unknown = value,
): RemoteContractError | UnknownRemoteError | LocalTransportError {
  const base = remoteBaseSchema.safeParse(value);
  if (!base.success) {
    return new LocalTransportError("malformed_response", "Ora returned a malformed error response", responseBody);
  }
  const known = contractErrorSchema.safeParse(value);
  if (known.success) {
    return new RemoteContractError(known.data, status, responseBody);
  }
  if (knownCodes.has(base.data.code)) {
    return new LocalTransportError("malformed_response", "Ora returned invalid parameters for a known error", responseBody);
  }
  return new UnknownRemoteError(base.data.code, base.data.requestId, status, responseBody);
}
