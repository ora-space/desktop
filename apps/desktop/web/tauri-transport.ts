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

const unsupportedOperations = {
  openProjectWorkContext: true,
  renewProjectWorkContext: true,
  listDirectory: true,
} as const satisfies Partial<Record<EndpointOperation, true>>;

type UnsupportedTauriOperation = keyof typeof unsupportedOperations;
type TauriStreamOperation = "loadSession" | "promptSession" | "watchSpecs" | "watchWorkspace";
type SupportedTauriOperation = Exclude<
  EndpointOperation,
  UnsupportedTauriOperation | TauriStreamOperation
>;

const tauriCommands = {
  // =============================================================================
  // project
  // =============================================================================
  createProject: "create_project",
  getProject: "get_project",
  listProjects: "list_projects",
  listProjectBranches: "list_project_branches",
  updateProject: "update_project",
  deleteProject: "delete_project",

  // =============================================================================
  // task
  // =============================================================================
  createTask: "create_task",
  getTask: "get_task",
  listTasks: "list_tasks",
  updateTask: "update_task",
  deleteTask: "delete_task",
  getTaskWorkspace: "get_task_workspace",
  getTaskDiff: "get_task_diff",
  commitTaskChanges: "commit_task_changes",
  pushTaskBranch: "push_task_branch",
  listTaskDiffComments: "list_task_diff_comments",
  createTaskDiffComment: "create_task_diff_comment",
  replyTaskDiffComment: "reply_task_diff_comment",
  setTaskDiffCommentStatus: "set_task_diff_comment_status",

  // =============================================================================
  // fileSystem
  // =============================================================================
  listWorkspaceDirectory: "list_workspace_directory",
  readWorkspaceFile: "read_workspace_file",
  searchWorkspace: "search_workspace",

  // =============================================================================
  // spec
  // =============================================================================
  getSpecCatalog: "get_spec_catalog",
  readSpec: "read_spec",
  resolveSpecSource: "resolve_spec_source",
  updateProjectSpecSources: "update_project_spec_sources",

  // =============================================================================
  // session
  // =============================================================================
  warmSession: "warm_session",
  setSessionConfig: "set_session_config",
  attachSession: "attach_session",
  getSession: "get_session",
  listSessions: "list_sessions",
  switchSessionAgent: "switch_session_agent",
  resumeSessionHistory: "resume_session_history",
  respondToSessionPermission: "respond_to_session_permission",
  stopSession: "stop_session",
  deleteSession: "delete_session",

  // =============================================================================
  // agentRuntime
  // =============================================================================
  getAgentRuntimeStatus: "get_agent_runtime_status",

  // =============================================================================
  // skill
  // =============================================================================
  createSkill: "create_skill",
  getSkill: "get_skill",
  listSkills: "list_skills",
  updateSkill: "update_skill",
  deleteSkill: "delete_skill",
  // =============================================================================
  // agent
  // =============================================================================
  prepareSkillImport: "prepare_skill_import",
  getSkillImport: "get_skill_import",
  commitSkillImport: "commit_skill_import",
  cancelSkillImport: "cancel_skill_import",
  prepareAgentImport: "prepare_agent_import",
  commitAgentImport: "commit_agent_import",
  createAgent: "create_agent",
  getAgent: "get_agent",
  listAgents: "list_agents",
  updateAgent: "update_agent",
  deleteAgent: "delete_agent",

  // =============================================================================
  // gitIdentity
  // =============================================================================
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
      if (operation in unsupportedOperations || isTauriStreamOperation(operation)) {
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
        if (error instanceof RemoteContractError || error instanceof UnknownRemoteError || error instanceof LocalTransportError) throw error;
        if (isAbortError(error)) throw transportError("cancelled", "Desktop command was cancelled");
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

/** Identifies operations that must use the shared Tauri channel stream command. */
function isTauriStreamOperation(operation: EndpointOperation): operation is TauriStreamOperation {
  return operation === "loadSession" || operation === "promptSession" || operation === "watchSpecs" || operation === "watchWorkspace";
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
        throw decodeRemoteError(frame.error, null, frame);
      }
      if (frame.type === "end") return;
    }
  } catch (error) {
    if (error instanceof RemoteContractError || error instanceof UnknownRemoteError || error instanceof LocalTransportError) throw error;
    if (isAbortError(error)) throw transportError("cancelled", "Desktop stream was cancelled");
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

function transportError(
  kind: ConstructorParameters<typeof LocalTransportError>[0],
  message: string,
): LocalTransportError {
  return new LocalTransportError(kind, message);
}

/** Builds the stable failure used for intentionally excluded Desktop operations. */
function unsupportedOperation(operationName: string): LocalTransportError {
  return transportError("unsupported_operation", `Desktop does not support operation ${operationName}`);
}

/** Normalizes serialized Rust command errors and opaque Tauri invocation failures. */
function normalizeInvokeError(error: unknown): Error {
  const decoded = decodeRemoteError(error, null, error);
  if (!(decoded instanceof LocalTransportError) || decoded.kind !== "malformed_response") {
    return decoded;
  }
  return new LocalTransportError("tauri_invoke_failure", "Desktop command invocation failed", error);
}
