import {
  tauriCommands,
  isTauriStreamOperation,
} from "./tauri-bindings.generated";
import { Channel, invoke } from "@tauri-apps/api/core";
import {
  LocalTransportError,
  RemoteContractError,
  UnknownRemoteError,
  decodeRemoteError,
  type ContractCallOptions,
  type ContractStreamFrame,
  type ContractTransport,
  type ContractTransportRequest,
  type EndpointOperation,
} from "@ora/contracts";

type TauriInvoke = <TResponse>(
  command: string,
  args: Record<string, unknown>,
) => Promise<TResponse>;
type ChannelLike<TEvent> = { onmessage: (event: TEvent) => void };
type ChannelFactory = <TEvent>() => ChannelLike<TEvent>;

const MAX_QUEUED_FRAMES = 256;

/** Creates the Desktop contracts transport backed by unary commands and Tauri IPC channels. */
export function createTauriTransport(
  invokeCommand: TauriInvoke = invoke,
  createChannel: ChannelFactory = () => new Channel(),
): ContractTransport {
  return {
    async send<TResponse>(
      request: ContractTransportRequest,
      options?: ContractCallOptions,
    ): Promise<TResponse> {
      const operation = request.operationName as EndpointOperation;
      if (isTauriStreamOperation(operation)) {
        throw transportError(
          "tauri_invoke_failure",
          `Stream operation ${operation} must use stream()`,
        );
      }
      const command = tauriCommands[operation];
      if (!Object.hasOwn(tauriCommands, operation)) {
        throw transportError(
          "tauri_invoke_failure",
          `Unknown unary operation ${operation}`,
        );
      }

      try {
        return await abortable(
          invokeCommand<TResponse>(command, { request: request.request }),
          options?.signal,
        );
      } catch (error) {
        if (
          error instanceof RemoteContractError ||
          error instanceof UnknownRemoteError ||
          error instanceof LocalTransportError
        )
          throw error;
        if (isAbortError(error))
          throw transportError("cancelled", "Desktop command was cancelled");
        throw normalizeInvokeError(error);
      }
    },
    stream<TEvent>(
      request: ContractTransportRequest,
      options?: ContractCallOptions,
    ): AsyncIterable<TEvent> {
      let consumed = false;
      return {
        [Symbol.asyncIterator](): AsyncIterator<TEvent> {
          if (consumed)
            throw transportError(
              "stream_already_consumed",
              "contract streams can only be consumed once",
            );
          consumed = true;
          return streamFromChannel<TEvent>(
            invokeCommand,
            createChannel,
            request,
            options,
          );
        },
      };
    },
  };
}

/** Starts one private channel stream and cancels its backend registration on every early exit. */
async function* streamFromChannel<TEvent>(
  invokeCommand: TauriInvoke,
  createChannel: ChannelFactory,
  request: ContractTransportRequest,
  options?: ContractCallOptions,
): AsyncGenerator<TEvent> {
  if (options?.signal?.aborted === true)
    throw transportError("cancelled", "Desktop stream was cancelled");
  if (!isTauriStreamOperation(request.operationName))
    throw transportError(
      "tauri_invoke_failure",
      `Unknown stream operation ${request.operationName}`,
    );
  const streamCallId = crypto.randomUUID();
  const channel = createChannel<ContractStreamFrame<TEvent>>();
  const frames: ContractStreamFrame<TEvent>[] = [];
  let overflowed = false;
  let wake: (() => void) | undefined;
  channel.onmessage = (frame) => {
    if (frames.length >= MAX_QUEUED_FRAMES) {
      overflowed = true;
      wake?.();
      wake = undefined;
      return;
    }
    frames.push(frame);
    wake?.();
    wake = undefined;
  };
  const abort = () => {
    // Signal creation too; finally repeats this after startup settles in case IPC delivery raced.
    void invokeCommand<void>("cancel_contract_stream", { streamCallId }).catch(
      () => undefined,
    );
    wake?.();
    wake = undefined;
  };
  options?.signal?.addEventListener("abort", abort, { once: true });

  try {
    await invokeCommand<void>("stream_contract", {
      operationName: request.operationName,
      request: request.request,
      streamCallId,
      onEvent: channel,
    });
    while (true) {
      if (isSignalAborted(options?.signal))
        throw abortError(options?.signal?.reason);
      if (overflowed) {
        throw transportError(
          "stream_queue_overflow",
          "contract stream consumer could not keep up with the backend",
        );
      }
      const frame = frames.shift();
      if (frame === undefined) {
        await new Promise<void>((resolve) => {
          wake = resolve;
        });
        continue;
      }
      if (frame.type === "data") yield frame.data;
      if (frame.type === "error") {
        throw decodeRemoteError(frame.error);
      }
      if (frame.type === "end") return;
    }
  } catch (error) {
    if (
      error instanceof RemoteContractError ||
      error instanceof UnknownRemoteError ||
      error instanceof LocalTransportError
    )
      throw error;
    if (isAbortError(error))
      throw transportError("cancelled", "Desktop stream was cancelled");
    throw normalizeInvokeError(error);
  } finally {
    options?.signal?.removeEventListener("abort", abort);
    channel.onmessage = () => undefined;
    await invokeCommand<void>("cancel_contract_stream", { streamCallId }).catch(
      () => undefined,
    );
  }
}

/** Rejects only the caller wait when a unary call is aborted; backend work is not rolled back. */
function abortable<T>(operation: Promise<T>, signal?: AbortSignal): Promise<T> {
  if (signal === undefined) return operation;
  if (signal.aborted) return Promise.reject(abortError(signal.reason));
  return new Promise<T>((resolve, reject) => {
    const abort = () => reject(abortError(signal.reason));
    signal.addEventListener("abort", abort, { once: true });
    operation
      .then(resolve, reject)
      .finally(() => signal.removeEventListener("abort", abort));
  });
}

function abortError(reason: unknown): DOMException {
  return new DOMException(
    typeof reason === "string" ? reason : "The operation was aborted",
    "AbortError",
  );
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError";
}

function isSignalAborted(signal: AbortSignal | undefined): boolean {
  return signal?.aborted === true;
}

function transportError(
  kind: ConstructorParameters<typeof LocalTransportError>[0],
  message: string,
): LocalTransportError {
  return new LocalTransportError(kind, message);
}

/** Normalizes serialized Rust command errors and opaque Tauri invocation failures. */
function normalizeInvokeError(error: unknown): Error {
  const decoded = decodeRemoteError(error);
  if (
    !(decoded instanceof LocalTransportError) ||
    decoded.kind !== "malformed_response"
  ) {
    return decoded;
  }
  return new LocalTransportError(
    "tauri_invoke_failure",
    "Desktop command invocation failed",
    error,
  );
}
