import type { JsonValue } from "../protocol.ts";
import {
  type AcpSender,
  AGENT_ACP,
  AGENT_LIST_MODELS,
  AGENT_START,
  AGENT_STOP,
  type AgentModel,
  type AgentStartContext,
  defineAgent,
} from "./contract.ts";

/**
 * Carries the process-level facts a plugin instance may need outside any agent session.
 *
 * The host hands session-scoped data (cwd, host version) to `onStart` instead, so this stays
 * limited to what is true for the whole plugin process.
 */
export interface PluginContext {
  readonly pluginId: string;
}

/**
 * Maps every class method onto the JSON-RPC method the Ora host invokes.
 *
 * The host contract fixes the wire names, so the mapping is explicit rather than derived from the
 * method name: a plugin that renamed `onListModels` would otherwise silently stop serving
 * `agent/listModels`.
 */
export const AGENT_METHOD_ROUTES = {
  onStart: AGENT_START,
  onStop: AGENT_STOP,
  onListModels: AGENT_LIST_MODELS,
} as const;

/** Maps the class method that consumes host notifications onto its wire name. */
export const AGENT_NOTIFICATION_ROUTES = {
  onAcp: AGENT_ACP,
} as const;

/**
 * Base class for a `kind: "agent"` plugin, exposing the agent contract as class methods.
 *
 * Required APIs are `abstract` so the compiler rejects an incomplete plugin, while optional APIs
 * ship a default implementation that can be overridden. Each method may also be mounted from a
 * separate module (`override onStart = handleStart`), which keeps a plugin with many APIs from
 * collapsing into one oversized entrypoint.
 */
export abstract class AgentPlugin {
  readonly type = "agent";

  // ------------------------- lifecycle hooks (optional) -------------------------

  /** Runs once before the plugin answers anything, for eager setup that must not race a call. */
  onActivate(_context: PluginContext): void | Promise<void> {}

  /** Runs once after the host closed the connection, for cleanup the host will not wait on. */
  onDeactivate(): void | Promise<void> {}

  // ------------------------- required agent contract ---------------------------

  /**
   * [agent/start] Brings the agent up so it can receive ACP frames.
   *
   * `send` stays valid for the rest of the process and is how every agent-originated ACP frame
   * reaches the host. Throw `PluginMethodError(AGENT_NOT_INSTALLED, ...)` when the underlying CLI
   * is missing, which the host treats as expected local configuration instead of a fault.
   */
  abstract onStart(
    context: AgentStartContext,
    send: AcpSender,
  ): void | Promise<void>;

  /** [agent/acp] Receives one ACP frame the host is forwarding to the agent. */
  abstract onAcp(frame: JsonValue): void | Promise<void>;

  /** [agent/listModels] Lists selectable models before any session exists. */
  abstract onListModels(): AgentModel[] | Promise<AgentModel[]>;

  // ------------------------- optional agent contract ---------------------------

  /** [agent/stop] Terminates the agent while leaving this plugin process alive. */
  onStop(): void | Promise<void> {}
}

/** One entry of the flattened dispatch table, already bound to its plugin instance. */
type BoundHandler = (...args: never[]) => unknown;

/**
 * Serves one agent plugin instance until the host shuts the process down.
 *
 * The instance is first flattened into a wire-name keyed table so dispatch never walks the
 * prototype chain, then adapted onto the SDK's agent definition, which owns the registration
 * handshake and the response shapes the host validates.
 */
export async function runAgentPlugin(
  plugin: AgentPlugin,
  context: PluginContext,
): Promise<void> {
  const routes = flattenRoutes(plugin);
  protectProtocolStdout();
  await plugin.onActivate(context);

  const definition = defineAgent({
    start: (startContext, send) =>
      invoke(routes, AGENT_METHOD_ROUTES.onStart, startContext, send) as
        | void
        | Promise<void>,
    stop: () =>
      invoke(routes, AGENT_METHOD_ROUTES.onStop) as void | Promise<void>,
    listModels: () =>
      invoke(routes, AGENT_METHOD_ROUTES.onListModels) as
        | AgentModel[]
        | Promise<AgentModel[]>,
    onAcp: (frame) =>
      invoke(routes, AGENT_NOTIFICATION_ROUTES.onAcp, frame) as
        | void
        | Promise<void>,
  });

  try {
    await definition.run();
  } finally {
    await plugin.onDeactivate();
  }
}

/**
 * Sends every console method to stderr before any plugin code can run.
 *
 * The SDK does the same when `run()` starts, but activation happens before that, and stdout is
 * the binary protocol channel: a single `console.log` in `onActivate` would be read by the host
 * as a corrupt frame and take the whole plugin down.
 */
function protectProtocolStdout(): void {
  const encoder = new TextEncoder();
  const write = (level: string, values: unknown[]) => {
    const rendered = values
      .map((value) => (typeof value === "string" ? value : Deno.inspect(value)))
      .join(" ");
    Deno.stderr.writeSync(encoder.encode(`[plugin:${level}] ${rendered}\n`));
  };
  console.debug = (...values: unknown[]) => write("debug", values);
  console.info = (...values: unknown[]) => write("info", values);
  console.log = (...values: unknown[]) => write("log", values);
  console.warn = (...values: unknown[]) => write("warn", values);
  console.error = (...values: unknown[]) => write("error", values);
}

/**
 * Collects every implemented API of one instance into a wire-name keyed dispatch table.
 *
 * Both class methods and instance fields are scanned, because mounting a handler module as a
 * field (`override onStart = handleStart`) is a supported way to organize a large plugin. Own
 * properties win over inherited ones, so an override always shadows the base implementation.
 */
function flattenRoutes(plugin: AgentPlugin): Map<string, BoundHandler> {
  const routes = new Map<string, BoundHandler>();
  const implemented = collectCallableNames(plugin);
  const source = plugin as unknown as Record<string, BoundHandler>;
  for (
    const [name, method] of Object.entries({
      ...AGENT_METHOD_ROUTES,
      ...AGENT_NOTIFICATION_ROUTES,
    })
  ) {
    if (!implemented.has(name)) {
      throw new Error(
        `Agent plugin does not implement ${name}, required for ${method}`,
      );
    }
    routes.set(method, source[name].bind(plugin));
  }
  return routes;
}

/** Walks the prototype chain once, returning the names that resolve to callable members. */
function collectCallableNames(instance: object): Set<string> {
  const names = new Set<string>();
  const source = instance as Record<string, unknown>;
  let current: object | null = instance;
  while (current !== null && current !== Object.prototype) {
    for (const name of Object.getOwnPropertyNames(current)) {
      if (name !== "constructor" && typeof source[name] === "function") {
        names.add(name);
      }
    }
    current = Object.getPrototypeOf(current);
  }
  return names;
}

/** Dispatches one wire method through the flattened table. */
function invoke(
  routes: Map<string, BoundHandler>,
  method: string,
  ...args: unknown[]
): unknown {
  const handler = routes.get(method);
  if (handler === undefined) {
    throw new Error(`Method '${method}' is not implemented by this plugin.`);
  }
  return handler(...(args as never[]));
}
