import { Channel, invoke } from "@tauri-apps/api/core";
import {
  ContractTransportError,
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

const unsupportedOperations = {
  openProjectWorkContext: true,
  renewProjectWorkContext: true,
  listDirectory: true,
} as const satisfies Partial<Record<EndpointOperation, true>>;

type UnsupportedTauriOperation = keyof typeof unsupportedOperations;
type SupportedTauriOperation = Exclude<
  EndpointOperation,
  UnsupportedTauriOperation | "loadSession" | "promptSession"
>;

const tauriCommands = {
  createProject: "create_project",
  getProject: "get_project",
  listProjects: "list_projects",
  updateProject: "update_project",
  deleteProject: "delete_project",
  createTask: "create_task",
  getTask: "get_task",
  listTasks: "list_tasks",
  updateTask: "update_task",
  deleteTask: "delete_task",
  createSession: "create_session",
  getSession: "get_session",
  listSessions: "list_sessions",
  respondToSessionPermission: "respond_to_session_permission",
  stopSession: "stop_session",
  deleteSession: "delete_session",
  createSkill: "create_skill",
  getSkill: "get_skill",
  listSkills: "list_skills",
  updateSkill: "update_skill",
  deleteSkill: "delete_skill",
  createAgent: "create_agent",
  getAgent: "get_agent",
  listAgents: "list_agents",
  updateAgent: "update_agent",
  deleteAgent: "delete_agent",
  getGitIdentity: "get_git_identity",
} as const satisfies Record<SupportedTauriOperation, string>;

/** Creates the Desktop contracts transport backed by unary commands and Tauri IPC channels. */
export function createTauriTransport(
  invokeCommand: TauriInvoke = invoke,
  createChannel: ChannelFactory = () => new Channel(),
): ContractTransport {
  return {
    async send<TResponse>(request: ContractTransportRequest, options?: ContractCallOptions): Promise<TResponse> {
      const operation = request.operationName as EndpointOperation;
      if (operation in unsupportedOperations || operation === "loadSession" || operation === "promptSession") {
        throw unsupportedOperation(operation);
      }
      const command = tauriCommands[operation as SupportedTauriOperation];
      if (!command) throw unsupportedOperation(request.operationName);

      try {
        return await abortable(
          invokeCommand<TResponse>(command, { request: request.request }),
          options?.signal,
        );
      } catch (error) {
        if (isAbortError(error) || error instanceof ContractTransportError) throw error;
        throw normalizeInvokeError(error);
      }
    },
    stream<TEvent>(request: ContractTransportRequest, options?: ContractCallOptions): AsyncIterable<TEvent> {
      let consumed = false;
      return {
        [Symbol.asyncIterator](): AsyncIterator<TEvent> {
          if (consumed) throw transportError("stream_already_consumed", "contract streams can only be consumed once");
          consumed = true;
          return streamFromChannel<TEvent>(invokeCommand, createChannel, request, options);
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
  if (options?.signal?.aborted === true) throw abortError(options.signal.reason);
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
      if (isSignalAborted(options?.signal)) throw abortError(options?.signal?.reason);
      if (overflowed) {
        throw transportError(
          "stream_queue_overflow",
          "contract stream consumer could not keep up with the backend",
        );
      }
      const frame = frames.shift();
      if (frame === undefined) {
        await new Promise<void>((resolve) => { wake = resolve; });
        continue;
      }
      if (frame.type === "data") yield frame.data;
      if (frame.type === "error") {
        throw new ContractTransportError({ ...frame.error, status: null, responseBody: frame });
      }
      if (frame.type === "end") return;
    }
  } catch (error) {
    if (isAbortError(error) || error instanceof ContractTransportError) throw error;
    throw normalizeInvokeError(error);
  } finally {
    options?.signal?.removeEventListener("abort", abort);
    await invokeCommand<void>("cancel_contract_stream", { streamCallId }).catch(() => undefined);
  }
}

/** Rejects only the caller wait when a unary call is aborted; backend work is not rolled back. */
function abortable<T>(operation: Promise<T>, signal?: AbortSignal): Promise<T> {
  if (signal === undefined) return operation;
  if (signal.aborted) return Promise.reject(abortError(signal.reason));
  return new Promise<T>((resolve, reject) => {
    const abort = () => reject(abortError(signal.reason));
    signal.addEventListener("abort", abort, { once: true });
    operation.then(resolve, reject).finally(() => signal.removeEventListener("abort", abort));
  });
}

function abortError(reason: unknown): DOMException {
  return new DOMException(typeof reason === "string" ? reason : "The operation was aborted", "AbortError");
}

function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === "AbortError";
}

function isSignalAborted(signal: AbortSignal | undefined): boolean {
  return signal?.aborted === true;
}

function transportError(code: string, message: string): ContractTransportError {
  return new ContractTransportError({ code, message, status: null, responseBody: null });
}

/** Builds the stable failure used for intentionally excluded Desktop operations. */
function unsupportedOperation(operationName: string): ContractTransportError {
  return transportError("unsupported_operation", `Desktop does not support operation ${operationName}`);
}

/** Normalizes serialized Rust command errors and opaque Tauri invocation failures. */
function normalizeInvokeError(error: unknown): ContractTransportError {
  if (isCommandError(error)) {
    return new ContractTransportError({ code: error.code, message: error.message, status: null, responseBody: error });
  }
  return new ContractTransportError({ code: "tauri_invoke_error", message: "Desktop command invocation failed", status: null, responseBody: error });
}

function isCommandError(error: unknown): error is { code: string; message: string } {
  if (typeof error !== "object" || error === null) return false;
  const record = error as Record<string, unknown>;
  return typeof record.code === "string" && typeof record.message === "string";
}
