import type { Readable, Writable } from "node:stream";

import type { AgentBusinessError, AgentEvent } from "@ora-space/plugin-sdk/agent";

import type { AgentTurnResult, InitializeParams } from "../generated/index.js";
import { FrameDecoder, MAX_FRAME_BYTES } from "../transport/frame.js";
import { ProtocolWriter } from "../transport/writer.js";
import {
  encodeError,
  encodeStream,
  encodeSuccess,
  parseInboundEnvelope,
  type RpcNotification,
  type RpcRequest,
} from "../rpc/envelope.js";
import {
  isAgentMethod,
  isOrdinaryMethod,
  validateActivateParams,
  validateAgentBusinessErrorData,
  validateAgentEvent,
  validateAgentRequest,
  validateAgentResult,
  validateCancelRequestParams,
  validateDeactivateParams,
  validateInitializeParams,
  validateTurnResult,
  type AgentMethodName,
} from "./contracts.js";
import { createBootstrapContext, type BootstrapContext } from "./context.js";
import {
  loadAndActivatePlugin,
  type DispatchProvider,
  type LoadedPlugin,
  type ModuleImporter,
} from "./loader.js";

const RUNTIME_VERSION = "1.0.0";
const ERROR_METHOD_NOT_FOUND = -32601;
const ERROR_INVALID_PARAMS = -32602;
const ERROR_INTERNAL = -32603;
const ERROR_AGENT_BUSINESS = -32000;
const ERROR_SERVER_BUSY = -32010;
const ERROR_REQUEST_CANCELLED = -32800;

export interface BootstrapIo {
  readonly stdin: Readable;
  readonly stdout: Writable;
  readonly stderr: Writable;
}

export interface BootstrapOptions {
  readonly importer?: ModuleImporter;
}

type SessionPhase =
  | "awaitingInitialize"
  | "initialized"
  | "running"
  | "deactivating"
  | "deactivated"
  | "exiting";

interface ActiveCall {
  readonly controller: AbortController;
  readonly method: AgentMethodName;
  readonly safety: boolean;
  task: Promise<unknown>;
}

interface ActiveTurn {
  readonly requestId: string;
  readonly providerId: string;
  readonly installationId: string;
  conversationId: string | undefined;
  task: Promise<AgentTurnResult | undefined>;
}

/** Owns one private bootstrap wire session and every author handler admitted into it. */
export class BootstrapSession {
  readonly #io: BootstrapIo;
  readonly #writer: ProtocolWriter;
  readonly #importer: ModuleImporter | undefined;
  readonly #active = new Map<string, ActiveCall>();
  readonly #turnsByRequest = new Map<string, ActiveTurn>();
  readonly #turnsByConversation = new Map<string, ActiveTurn>();
  readonly #conversationStops = new Map<string, Promise<"accepted" | "alreadyStopped">>();
  #phase: SessionPhase = "awaitingInitialize";
  #initialize: InitializeParams | undefined;
  #context: BootstrapContext | undefined;
  #loaded: LoadedPlugin | undefined;
  #highestRequestSequence = -1;
  #ordinaryActive = 0;
  #safetyActive = 0;
  #exitRequested = false;
  #fatalReject: (reason: unknown) => void = () => undefined;
  readonly #fatal: Promise<never>;

  constructor(io: BootstrapIo, options: BootstrapOptions = {}) {
    this.#io = io;
    this.#importer = options.importer;
    this.#writer = new ProtocolWriter(io.stdout, {
      maximumFrames: 256,
      maximumBytes: 16 * 1024 * 1024,
      reservedControlFrames: 16,
      reservedControlBytes: 1024 * 1024,
      reservedSafetyFrames: 16,
      reservedSafetyBytes: 1024 * 1024,
    });
    this.#fatal = new Promise<never>((_resolve, reject) => {
      this.#fatalReject = reject;
    });
  }

  /** Reads until the exact exit notification, treating EOF or any router failure as fatal. */
  async run(): Promise<void> {
    await Promise.race([this.#readLoop(), this.#fatal]);
    await Promise.allSettled([...this.#active.values()].map(({ task }) => task));
    await this.#writer.close();
  }

  async #readLoop(): Promise<void> {
    const decoder = new FrameDecoder(MAX_FRAME_BYTES);
    for await (const chunk of this.#io.stdin) {
      const bytes = typeof chunk === "string" ? new TextEncoder().encode(chunk) : new Uint8Array(chunk);
      for (const frame of decoder.decodeChunk(bytes)) {
        const envelope = parseInboundEnvelope(frame);
        if (envelope.type === "request") {
          await this.#acceptRequest(envelope);
        } else {
          await this.#acceptNotification(envelope);
        }
        if (this.#exitRequested) {
          return;
        }
      }
    }
    decoder.finish();
    throw new Error("Host stdin reached EOF before $/exit");
  }

  async #acceptRequest(request: RpcRequest): Promise<void> {
    this.#acceptMonotonicHostId(request.id);
    if (this.#phase === "awaitingInitialize") {
      if (request.method !== "$/initialize" || request.params === undefined) {
        throw new Error("the first Host frame must be an initialize Request");
      }
      await this.#initializeSession(request.id, request.params);
      return;
    }
    if (request.method === "$/initialize") {
      throw new Error("initialize cannot be repeated");
    }
    if (request.method === "$/activate") {
      await this.#activateSession(request);
      return;
    }
    if (request.method === "$/deactivate") {
      await this.#deactivateSession(request);
      return;
    }
    if (!isAgentMethod(request.method)) {
      await this.#writer.response(
        encodeError(request.id, ERROR_METHOD_NOT_FOUND, "Method not found"),
        "control",
      );
      return;
    }
    await this.#acceptAgentRequest(request, request.method);
  }

  async #acceptNotification(notification: RpcNotification): Promise<void> {
    switch (notification.method) {
      case "$/cancelRequest": {
        if (notification.params === undefined) {
          throw new Error("cancelRequest requires params");
        }
        const id = validateCancelRequestParams(notification.params);
        this.#active.get(id)?.controller.abort();
        return;
      }
      case "$/exit":
        if (notification.params !== undefined) {
          throw new Error("exit notification must omit params");
        }
        this.#phase = "exiting";
        for (const call of this.#active.values()) {
          call.controller.abort();
        }
        this.#exitRequested = true;
        return;
      default:
        throw new Error("Host sent an unsupported notification");
    }
  }

  async #initializeSession(id: string, params: Record<string, unknown>): Promise<void> {
    const initialize = validateInitializeParams(params);
    if (initialize.runtimeVersion !== RUNTIME_VERSION) {
      throw new Error("Host/private bootstrap runtime version mismatch");
    }
    this.#initialize = initialize;
    this.#context = createBootstrapContext(initialize, this.#io.stderr);
    await this.#writer.response(
      encodeSuccess(id, {
        wireVersion: 1,
        runtimeVersion: RUNTIME_VERSION,
        sessionId: initialize.sessionId,
        plugin: { id: initialize.plugin.id, version: initialize.plugin.version },
      }),
      "control",
    );
    this.#phase = "initialized";
  }

  async #activateSession(request: RpcRequest): Promise<void> {
    if (this.#phase !== "initialized" || request.params === undefined) {
      throw new Error("activate is invalid in the current lifecycle phase");
    }
    validateActivateParams(request.params);
    const initialize = this.#requireInitialize();
    const context = this.#requireContext();
    try {
      this.#loaded = await loadAndActivatePlugin(
        initialize.paths.entryPath,
        initialize.sessionId,
        context.extensionContext,
        initialize.declaredAgents,
        this.#importer,
      );
      const providers = [...this.#loaded.providers.values()]
        .map(({ id, contractVersion }) => ({ id, contractVersion }))
        .sort((left, right) => left.id.localeCompare(right.id));
      await this.#writer.response(encodeSuccess(request.id, { providers }), "control");
      this.#phase = "running";
    } catch {
      await this.#cleanupAfterFailedActivation();
      await this.#writer.response(
        encodeError(request.id, ERROR_INTERNAL, "Plugin activation failed"),
        "control",
      );
      this.#fail(new Error("plugin activation failed"));
    }
  }

  async #deactivateSession(request: RpcRequest): Promise<void> {
    if (this.#phase !== "running" || request.params === undefined) {
      throw new Error("deactivate is invalid in the current lifecycle phase");
    }
    validateDeactivateParams(request.params);
    this.#phase = "deactivating";
    for (const call of this.#active.values()) {
      call.controller.abort();
    }
    await Promise.allSettled([...this.#active.values()].map(({ task }) => task));

    const loaded = this.#loaded;
    const context = this.#requireContext();
    context.shutdown.abort();
    let cleanupFailed = false;
    try {
      await loaded?.definition.deactivate?.();
    } catch {
      cleanupFailed = true;
    }
    try {
      await context.subscriptions.disposeAll();
    } catch {
      cleanupFailed = true;
    }
    if (cleanupFailed) {
      await this.#writer.response(
        encodeError(request.id, ERROR_INTERNAL, "Plugin deactivation failed"),
        "control",
      );
    } else {
      await this.#writer.response(encodeSuccess(request.id, {}), "control");
    }
    this.#phase = "deactivated";
  }

  async #acceptAgentRequest(request: RpcRequest, method: AgentMethodName): Promise<void> {
    if (this.#phase !== "running" || request.params === undefined) {
      await this.#writer.response(
        encodeError(request.id, ERROR_INVALID_PARAMS, "Invalid params"),
        method === "agent.cancelConversation" ? "safety" : "ordinary",
      );
      return;
    }
    let params: Record<string, unknown>;
    try {
      params = validateAgentRequest(method, request.params, this.#requireInitialize().limits);
      if (!this.#requireLoaded().providers.has(params.providerId as string)) {
        throw new TypeError("providerId is not declared by this plugin");
      }
    } catch {
      await this.#writer.response(
        encodeError(request.id, ERROR_INVALID_PARAMS, "Invalid params"),
        method === "agent.cancelConversation" ? "safety" : "ordinary",
      );
      return;
    }

    const limits = this.#requireInitialize().limits;
    const safety = !isOrdinaryMethod(method);
    if (!safety && this.#ordinaryActive >= limits.maxPendingRequests) {
      await this.#writer.response(
        encodeError(request.id, ERROR_SERVER_BUSY, "Server busy"),
        "ordinary",
      );
      return;
    }
    const streaming = method === "agent.startConversation" || method === "agent.sendMessage";
    if (streaming && this.#turnsByRequest.size >= limits.maxActiveTurns) {
      await this.#writer.response(
        encodeError(request.id, ERROR_SERVER_BUSY, "Server busy"),
        "ordinary",
      );
      return;
    }
    if (safety && this.#safetyActive >= limits.maxActiveTurns) {
      this.#fail(new Error("safety executor reserve exhausted"));
      return;
    }

    let turn: ActiveTurn | undefined;
    if (streaming) {
      const conversationId =
        method === "agent.sendMessage" ? (params.conversationId as string) : undefined;
      turn = {
        requestId: request.id,
        providerId: params.providerId as string,
        installationId: params.installationId as string,
        conversationId,
        task: Promise.resolve(undefined),
      };
      if (conversationId !== undefined) {
        const key = conversationKey(turn.providerId, turn.installationId, conversationId);
        if (this.#turnsByConversation.has(key)) {
          await this.#writer.response(
            encodeError(request.id, ERROR_SERVER_BUSY, "Conversation already has an active turn"),
            "ordinary",
          );
          return;
        }
        this.#turnsByConversation.set(key, turn);
      }
      this.#turnsByRequest.set(request.id, turn);
    }

    const controller = new AbortController();
    const call: ActiveCall = {
      controller,
      method,
      safety,
      task: Promise.resolve(),
    };
    if (safety) {
      this.#safetyActive += 1;
    } else {
      this.#ordinaryActive += 1;
    }
    this.#active.set(request.id, call);
    const task =
      method === "agent.cancelConversation"
        ? this.#runCancelConversation(request.id, params, controller)
        : this.#runAgentRequest(request.id, method, params, controller, turn);
    if (turn !== undefined) {
      turn.task = task;
    }
    call.task = task
      .catch((error: unknown) => this.#fail(error))
      .finally(() => {
        this.#active.delete(request.id);
        if (turn !== undefined) {
          this.#turnsByRequest.delete(turn.requestId);
          if (turn.conversationId !== undefined) {
            const key = conversationKey(
              turn.providerId,
              turn.installationId,
              turn.conversationId,
            );
            if (this.#turnsByConversation.get(key) === turn) {
              this.#turnsByConversation.delete(key);
            }
          }
        }
        if (safety) {
          this.#safetyActive -= 1;
        } else {
          this.#ordinaryActive -= 1;
        }
      });
  }

  async #runAgentRequest(
    id: string,
    method: AgentMethodName,
    params: Record<string, unknown>,
    controller: AbortController,
    turn: ActiveTurn | undefined,
  ): Promise<AgentTurnResult | undefined> {
    const provider = this.#provider(params.providerId as string);
    const call = Object.freeze({ requestId: id, signal: controller.signal });
    const lane = method === "agent.cancelConversation" ? "safety" : "ordinary";
    try {
      const returned = provider.handlers[method](call, Object.freeze(params));
      if (method === "agent.startConversation" || method === "agent.sendMessage") {
        if (turn === undefined) {
          throw new AgentContractViolationError("streaming request has no active-turn owner");
        }
        return await this.#driveGenerator(id, method, params, returned, turn);
      } else {
        const value = await returned;
        const requestedLimit = typeof params.limit === "number" ? params.limit : undefined;
        const result = contractValue(() =>
          validateAgentResult(
            method,
            value,
            this.#requireInitialize().limits,
            requestedLimit,
          ),
        );
        await this.#writer.response(encodeSuccess(id, result), lane);
      }
    } catch (error) {
      if (error instanceof AgentContractViolationError) {
        throw error;
      }
      if (controller.signal.aborted) {
        await this.#writer.response(
          encodeError(id, ERROR_REQUEST_CANCELLED, "Request cancelled"),
          lane,
        );
      } else if (this.#isBusinessError(error)) {
        const business = error as AgentBusinessError;
        const data = contractValue(() =>
          validateAgentBusinessErrorData(
            {
              kind: business.kind,
              retryable: business.retryable,
              ...(business.details === undefined ? {} : { details: business.details }),
            },
            this.#requireInitialize().limits,
          ),
        );
        await this.#writer.response(
          encodeError(id, ERROR_AGENT_BUSINESS, business.message, data),
          lane,
        );
      } else {
        await this.#writer.response(
          encodeError(id, ERROR_AGENT_BUSINESS, "Agent provider failed", {
            kind: "providerFailure",
            retryable: false,
          }),
          lane,
        );
      }
    }
    return undefined;
  }

  /** Joins one safety action and proves its response follows the target turn's terminal frame. */
  async #runCancelConversation(
    id: string,
    params: Record<string, unknown>,
    controller: AbortController,
  ): Promise<undefined> {
    const providerId = params.providerId as string;
    const installationId = params.installationId as string;
    const conversationId = params.conversationId as string;
    const key = conversationKey(providerId, installationId, conversationId);
    const target = this.#turnsByConversation.get(key);
    if (target === undefined) {
      await this.#writer.response(
        encodeSuccess(id, { disposition: "alreadyStopped" }),
        "safety",
      );
      return undefined;
    }

    let stop = this.#conversationStops.get(key);
    if (stop === undefined) {
      stop = this.#executeConversationCancel(id, params, controller, target);
      this.#conversationStops.set(key, stop);
      void stop
        .finally(() => {
          if (this.#conversationStops.get(key) === stop) {
            this.#conversationStops.delete(key);
          }
        })
        .catch(() => undefined);
    }
    const disposition = await stop;
    await this.#writer.response(encodeSuccess(id, { disposition }), "safety");
    return undefined;
  }

  /** Executes the provider cancellation once and verifies the target terminal disposition. */
  async #executeConversationCancel(
    id: string,
    params: Record<string, unknown>,
    controller: AbortController,
    target: ActiveTurn,
  ): Promise<"accepted" | "alreadyStopped"> {
    const provider = this.#provider(params.providerId as string);
    const call = Object.freeze({ requestId: id, signal: controller.signal });
    const returned = provider.handlers["agent.cancelConversation"](
      call,
      Object.freeze(params),
    );
    const value = await returned;
    const result = contractValue(() =>
      validateAgentResult(
        "agent.cancelConversation",
        value,
        this.#requireInitialize().limits,
      ),
    );
    const disposition = result.disposition as "accepted" | "alreadyStopped";
    const terminal = await target.task;
    if (
      terminal === undefined ||
      (disposition === "accepted" && terminal.finishReason !== "cancelled") ||
      (disposition === "alreadyStopped" &&
        terminal.finishReason !== "completed" &&
        terminal.finishReason !== "limit")
    ) {
      throw new AgentContractViolationError(
        "business cancellation result does not match the target turn terminal",
      );
    }
    return disposition;
  }

  async #driveGenerator(
    id: string,
    method: "agent.startConversation" | "agent.sendMessage",
    params: Record<string, unknown>,
    returned: unknown,
    turn: ActiveTurn,
  ): Promise<AgentTurnResult> {
    if (returned === null || typeof returned !== "object") {
      throw new AgentContractViolationError("streaming handler must return an AsyncGenerator");
    }
    const next = contractValue(
      () => findDataFunction(returned, "next"),
    ) as (this: object) => Promise<IteratorResult<AgentEvent, unknown>>;
    let sequence = 0;
    let conversationId = method === "agent.sendMessage" ? (params.conversationId as string) : undefined;
    for (;;) {
      const step = await next.call(returned);
      if (step === null || typeof step !== "object" || typeof step.done !== "boolean") {
        throw new AgentContractViolationError("generator returned an invalid iterator result");
      }
      if (step.done) {
        const result = contractValue(() =>
          validateTurnResult(step.value, this.#requireInitialize().limits),
        );
        if (conversationId === undefined || result.conversationId !== conversationId) {
          throw new AgentContractViolationError(
            "generator terminal conversation correlation failed",
          );
        }
        await this.#writer.response(encodeSuccess(id, result), "ordinary");
        return result;
      }
      const event = contractValue(() =>
        validateAgentEvent(step.value, this.#requireInitialize().limits),
      );
      if (event.kind === "conversationStarted") {
        if (method === "agent.sendMessage" || conversationId !== undefined) {
          throw new AgentContractViolationError(
            "generator emitted an unexpected conversationStarted event",
          );
        }
        conversationId = event.conversationId;
        this.#bindConversationTurn(turn, conversationId);
      } else if (method === "agent.startConversation" && conversationId === undefined) {
        throw new AgentContractViolationError(
          "startConversation must identify the conversation before other events",
        );
      }
      sequence += 1;
      await this.#writer.notification(encodeStream(id, sequence, event), "ordinary");
    }
  }

  /** Atomically replaces a provisional start request with its plugin-produced conversation key. */
  #bindConversationTurn(turn: ActiveTurn, conversationId: string): void {
    const key = conversationKey(turn.providerId, turn.installationId, conversationId);
    if (this.#turnsByConversation.has(key)) {
      throw new AgentContractViolationError("conversation already has an active turn");
    }
    turn.conversationId = conversationId;
    this.#turnsByConversation.set(key, turn);
  }

  #acceptMonotonicHostId(id: string): void {
    if (!/^h:(?:0|[1-9][0-9]*)$/u.test(id)) {
      throw new Error("Host request id is invalid");
    }
    const sequence = Number(id.slice(2));
    if (!Number.isSafeInteger(sequence) || sequence <= this.#highestRequestSequence) {
      throw new Error("Host request id was reused or is not monotonic");
    }
    this.#highestRequestSequence = sequence;
  }

  #provider(id: string): DispatchProvider {
    const provider = this.#requireLoaded().providers.get(id);
    if (provider === undefined) {
      throw new TypeError("provider is not active");
    }
    return provider;
  }

  #isBusinessError(error: unknown): boolean {
    return typeof error === "object" && error !== null && this.#requireContext().businessErrors.has(error);
  }

  #requireInitialize(): InitializeParams {
    if (this.#initialize === undefined) {
      throw new Error("bootstrap is not initialized");
    }
    return this.#initialize;
  }

  #requireContext(): BootstrapContext {
    if (this.#context === undefined) {
      throw new Error("bootstrap context is unavailable");
    }
    return this.#context;
  }

  #requireLoaded(): LoadedPlugin {
    if (this.#loaded === undefined) {
      throw new Error("plugin dispatch is unavailable");
    }
    return this.#loaded;
  }

  async #cleanupAfterFailedActivation(): Promise<void> {
    const context = this.#requireContext();
    context.shutdown.abort();
    try {
      await context.subscriptions.disposeAll();
    } catch {
      // The activation failure remains primary; disposal failure is intentionally secondary.
    }
  }

  #fail(error: unknown): void {
    for (const call of this.#active.values()) {
      call.controller.abort();
    }
    this.#fatalReject(error instanceof Error ? error : new Error("bootstrap runtime failed"));
  }
}

class AgentContractViolationError extends Error {}

/** Converts validator failures into a distinct fatal category without classifying provider throws. */
function contractValue<T>(validate: () => T): T {
  try {
    return validate();
  } catch (error) {
    throw new AgentContractViolationError(
      error instanceof Error ? error.message : "Agent contract validation failed",
    );
  }
}

/** Builds an unambiguous generation-local active-turn key from validated opaque identities. */
function conversationKey(providerId: string, installationId: string, conversationId: string): string {
  return JSON.stringify([providerId, installationId, conversationId]);
}

/** Finds a generator method without invoking accessors on an author-controlled prototype. */
function findDataFunction(object: object, name: string): CallableFunction {
  let current: object | null = object;
  while (current !== null) {
    const descriptor = Object.getOwnPropertyDescriptor(current, name);
    if (descriptor !== undefined) {
      if (!("value" in descriptor) || typeof descriptor.value !== "function") {
        throw new TypeError(`${name} must be a data function`);
      }
      return descriptor.value as CallableFunction;
    }
    current = Object.getPrototypeOf(current) as object | null;
  }
  throw new TypeError(`${name} data function is missing`);
}

/** Prevents author code and console helpers from writing unframed bytes to protocol stdout. */
export function installStdoutGuard(stderr: Writable): void {
  const stderrWrite = stderr.write.bind(stderr);
  const format = (values: readonly unknown[]): string =>
    values
      .map((value) => {
        try {
          return typeof value === "string" ? value : String(value);
        } catch {
          return "<unprintable>";
        }
      })
      .join(" ")
      .slice(0, 8192);
  for (const method of ["debug", "info", "log", "warn", "error"] as const) {
    console[method] = (...values: unknown[]) => {
      stderrWrite(`[plugin:console] ${format(values)}\n`);
    };
  }
  process.stdout.write = (() => {
    throw new Error("process.stdout is reserved for Ora protocol frames");
  }) as typeof process.stdout.write;
}
