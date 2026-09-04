import { createPlugin, type Plugin, PluginMethodError } from "./plugin.ts";
import {
  INVALID_PARAMS,
  type JsonValue,
  type WorkbenchCallParams,
} from "./protocol/index.ts";
import { createStorage, type PluginStorage } from "./storage.ts";
import type { PluginInvocationContext } from "./context.ts";

/** One method call from a workbench page, with the page-supplied input. */
export interface WorkbenchCall<Input = JsonValue> {
  /** Host-issued authority for contextual APIs; opaque to both page and plugin. */
  context: PluginInvocationContext;
  input: Input;
}

/** A single method handler: it receives the host envelope and returns the page's result. */
export type WorkbenchMethod = (
  call: WorkbenchCall,
) => JsonValue | Promise<JsonValue>;

/** The methods a workbench plugin exposes to its page, keyed by method name. */
export interface WorkbenchPluginDefinition {
  methods: Record<string, WorkbenchMethod>;
}

/** A workbench plugin ready to run, with the host channels it may use while running. */
export interface WorkbenchPlugin {
  readonly plugin: Plugin;
  readonly storage: PluginStorage;
  /** Announces the registration and serves Ora until shutdown; see `Plugin.run`. */
  run(...args: Parameters<Plugin["run"]>): Promise<void>;
}

/**
 * Builds a workbench plugin whose page-callable methods are exactly the keys of `methods`.
 *
 * Each key is registered as a plugin method; the host wraps the page's params in the envelope
 * `{ context: { id }, input }`, which this SDK unpacks into a
 * {@link WorkbenchCall} so plugin code never parses the envelope. The v1 contract has no
 * plugin-to-page channel, so no notification is ever declared: a `main.js` only answers calls.
 *
 * The registered set may be a superset or subset of the manifest `[workbench].methods`; the host
 * intersects the two, so a method registered here but not declared in the manifest is simply
 * never reachable from the page, and a declared method missing here is reported as unavailable.
 */
export function defineWorkbenchPlugin(
  definition: WorkbenchPluginDefinition,
): WorkbenchPlugin {
  const plugin = createPlugin();
  for (const [name, handler] of Object.entries(definition.methods)) {
    plugin.registerMethod(name, async (params) => {
      return await handler(parseCall(name, params));
    });
  }

  return {
    plugin,
    storage: createStorage(plugin),
    run: (...args) => plugin.run(...args),
  };
}

/** Unpacks the host envelope of one workbench call. */
function parseCall(method: string, params: JsonValue): WorkbenchCall {
  const context = isRecord(params) ? params.context : undefined;
  if (
    !isRecord(context) || typeof context.id !== "string" ||
    context.id.length === 0
  ) {
    throw new PluginMethodError(
      INVALID_PARAMS,
      `${method} was not called with a valid invocation context`,
    );
  }
  const envelope = params as WorkbenchCallParams;
  return {
    context: envelope.context,
    input: envelope.input ?? null,
  };
}

function isRecord(value: unknown): value is Record<string, JsonValue> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
