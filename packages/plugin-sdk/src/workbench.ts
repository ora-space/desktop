import { createPlugin, type Plugin, PluginMethodError } from "./plugin.ts";
import type { JsonValue } from "./protocol.ts";
import { createStorage, type PluginStorage } from "./storage.ts";

/**
 * Identifies which page instance, on which process generation, a workbench call came from.
 *
 * The host fills these in from the calling webview; a `main.js` needs them only to tell its own
 * multiple open pages apart. They never widen what the call may do.
 */
export interface WorkbenchSurface {
  instanceId: number;
  /** Generation of the plugin process Ora addressed for this call. */
  generation: number;
}

/** One method call from a workbench page, with the page-supplied input. */
export interface WorkbenchCall<Input = JsonValue> {
  surface: WorkbenchSurface;
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
 * `{ surface: { instance_id, generation }, input }`, which this SDK unpacks into a
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
  const surface = isRecord(params) ? params.surface : undefined;
  if (
    !isRecord(surface) || typeof surface.instance_id !== "number" ||
    typeof surface.generation !== "number"
  ) {
    throw new PluginMethodError(
      -32602,
      `${method} was not called with a valid surface envelope`,
    );
  }
  return {
    surface: {
      instanceId: surface.instance_id,
      generation: surface.generation,
    },
    input: (isRecord(params) ? params.input : undefined) ?? null,
  };
}

function isRecord(value: unknown): value is Record<string, JsonValue> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
