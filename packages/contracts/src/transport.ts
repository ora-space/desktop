import { z } from "zod";
import type { ContractError } from "./error.js";
import { contractErrorSchema, publicErrorSchema } from "./error.schema.js";

export type ContractTransportRequest = {
  operationName: string;
  request: unknown;
};

export type ContractCallOptions = {
  readonly signal?: AbortSignal;
};

export interface ContractTransport {
  send<TResponse>(
    request: ContractTransportRequest,
    options?: ContractCallOptions,
  ): Promise<TResponse>;
  stream<TEvent>(
    request: ContractTransportRequest,
    options?: ContractCallOptions,
  ): AsyncIterable<TEvent>;
}

export type ContractStreamFrame<TEvent> =
  | { type: "data"; data: TEvent }
  | { type: "error"; error: unknown }
  | { type: "end" };

export const localTransportErrorKinds = [
  "tauri_invoke_failure",
  "malformed_response",
  "stream_queue_overflow",
  "stream_already_consumed",
  "cancelled",
] as const;

export type LocalTransportErrorKind = (typeof localTransportErrorKinds)[number];

/** Carries one runtime-validated error produced by an Ora adapter. */
export class RemoteContractError extends Error {
  readonly payload: ContractError;
  readonly rawPayload: unknown;

  constructor(payload: ContractError, rawPayload: unknown) {
    super(
      `Remote Ora request failed with ${payload.code} (${payload.requestId})`,
    );
    this.name = "RemoteContractError";
    this.payload = payload;
    this.rawPayload = rawPayload;
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
  readonly rawPayload: unknown;

  constructor(rawCode: string, requestId: string, rawPayload: unknown) {
    super(`Remote Ora request returned unknown code ${rawCode} (${requestId})`);
    this.name = "UnknownRemoteError";
    this.rawCode = rawCode;
    this.requestId = requestId;
    this.rawPayload = rawPayload;
  }
}

/** Represents a finite failure of the local Desktop transport itself. */
export class LocalTransportError extends Error {
  readonly kind: LocalTransportErrorKind;
  readonly causeValue: unknown;

  constructor(
    kind: LocalTransportErrorKind,
    technicalMessage: string,
    causeValue: unknown = null,
  ) {
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

/** Validates one IPC payload and distinguishes known, unknown, and malformed errors. */
export function decodeRemoteError(
  value: unknown,
): RemoteContractError | UnknownRemoteError | LocalTransportError {
  const base = remoteBaseSchema.safeParse(value);
  if (!base.success) {
    return new LocalTransportError(
      "malformed_response",
      "Ora returned a malformed error response",
      value,
    );
  }
  const known = contractErrorSchema.safeParse(value);
  if (known.success) {
    return new RemoteContractError(known.data, value);
  }
  if (knownCodes.has(base.data.code)) {
    return new LocalTransportError(
      "malformed_response",
      "Ora returned invalid parameters for a known error",
      value,
    );
  }
  return new UnknownRemoteError(
    base.data.code,
    base.data.requestId,
    value,
  );
}
