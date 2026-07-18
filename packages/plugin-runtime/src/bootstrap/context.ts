import type { Writable } from "node:stream";

import type {
  AgentBusinessError,
  AgentBusinessErrorInput,
  AuthorBusinessFailureKind,
  Disposable,
  ExtensionContext,
  PluginLogger,
  SubscriptionStore,
} from "@ora-space/plugin-sdk/agent";

import type { InitializeParams } from "../generated/index.js";
import { requireBoundedString, requirePlainObject, utf8Bytes } from "../rpc/envelope.js";
import { validatePlainJson } from "./contracts.js";

const AUTHOR_FAILURES = new Set<AuthorBusinessFailureKind>([
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
]);

export interface BootstrapContext {
  readonly extensionContext: ExtensionContext;
  readonly shutdown: AbortController;
  readonly businessErrors: WeakSet<object>;
  readonly subscriptions: SubscriptionRegistry;
}

/** Builds the per-generation author context and private business-error brand. */
export function createBootstrapContext(
  initialize: InitializeParams,
  stderr: Writable,
): BootstrapContext {
  const shutdown = new AbortController();
  const businessErrors = new WeakSet<object>();
  const subscriptions = new SubscriptionRegistry();
  const logger = createLogger(stderr);
  const errors = Object.freeze({
    business(input: AgentBusinessErrorInput): AgentBusinessError {
      const object = requirePlainObject(input, "business error input");
      const kind = object.kind;
      if (typeof kind !== "string" || !AUTHOR_FAILURES.has(kind as AuthorBusinessFailureKind)) {
        throw new TypeError("business error kind is invalid or reserved");
      }
      const message = requireBoundedString(object.message, "business error message", 8192);
      const retryable = object.retryable === undefined ? false : object.retryable;
      if (typeof retryable !== "boolean") {
        throw new TypeError("business error retryable must be boolean");
      }
      const details = object.details;
      if (details !== undefined) {
        validatePlainJson(details);
        const encoded = JSON.stringify(details);
        if (encoded === undefined || utf8Bytes(encoded) > 64 * 1024) {
          throw new TypeError("business error details exceed their byte cap");
        }
      }

      const error = new Error(message) as AgentBusinessError;
      Object.defineProperties(error, {
        name: { value: "AgentBusinessError", enumerable: true },
        kind: { value: kind, enumerable: true },
        retryable: { value: retryable, enumerable: true },
        ...(details === undefined ? {} : { details: { value: details, enumerable: true } }),
      });
      businessErrors.add(error);
      return error;
    },
  });

  const extensionContext: ExtensionContext = Object.freeze({
    plugin: Object.freeze({ id: initialize.plugin.id, version: initialize.plugin.version }),
    sessionId: initialize.sessionId,
    extensionPath: initialize.paths.extensionPath,
    storagePath: initialize.paths.storagePath,
    logger,
    shutdownSignal: shutdown.signal,
    subscriptions,
    errors,
  });
  return { extensionContext, shutdown, businessErrors, subscriptions };
}

/** Owns author disposables and executes them once in reverse registration order. */
export class SubscriptionRegistry implements SubscriptionStore {
  readonly #subscriptions: Disposable[] = [];
  readonly #registered = new Set<Disposable>();
  #disposed = false;

  add<T extends Disposable>(disposable: T): T {
    if (this.#disposed) {
      throw new Error("subscription store is already disposed");
    }
    if (disposable === null || typeof disposable !== "object" || typeof disposable.dispose !== "function") {
      throw new TypeError("subscription must expose dispose()");
    }
    if (this.#registered.has(disposable)) {
      throw new TypeError("subscription is already registered");
    }
    this.#registered.add(disposable);
    this.#subscriptions.push(disposable);
    return disposable;
  }

  /** Attempts every disposal even when earlier entries fail, then reports the first failure. */
  async disposeAll(perSubscriptionTimeoutMs = 2_000): Promise<void> {
    if (this.#disposed) {
      return;
    }
    this.#disposed = true;
    let firstFailure: unknown;
    for (const subscription of this.#subscriptions.reverse()) {
      try {
        await withTimeout(Promise.resolve(subscription.dispose()), perSubscriptionTimeoutMs);
      } catch (error) {
        firstFailure ??= error;
      }
    }
    this.#subscriptions.length = 0;
    this.#registered.clear();
    if (firstFailure !== undefined) {
      throw firstFailure;
    }
  }
}

/** Redirects bounded console-style messages to stderr without accumulating on backpressure. */
function createLogger(stderr: Writable): PluginLogger {
  const write = stderr.write.bind(stderr);
  let writable = true;
  stderr.on("drain", () => {
    writable = true;
  });
  const log = (level: string, message: string): void => {
    if (!writable) {
      return;
    }
    const bounded = redact(requireBoundedString(message, "log message", 8192));
    writable = write(`[plugin:${level}] ${bounded}\n`);
  };
  return Object.freeze({
    debug: (message: string) => log("debug", message),
    info: (message: string) => log("info", message),
    warn: (message: string) => log("warn", message),
    error: (message: string) => log("error", message),
  });
}

function redact(message: string): string {
  return message
    .replace(/(authorization|token|secret|password)\s*[:=]\s*[^\s,;]+/giu, "$1=<redacted>")
    .replace(/Bearer\s+[^\s]+/giu, "Bearer <redacted>");
}

async function withTimeout<T>(promise: Promise<T>, timeoutMs: number): Promise<T> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_resolve, reject) => {
    timer = setTimeout(() => reject(new Error("plugin cleanup phase timed out")), timeoutMs);
  });
  try {
    return await Promise.race([promise, timeout]);
  } finally {
    if (timer !== undefined) {
      clearTimeout(timer);
    }
  }
}
