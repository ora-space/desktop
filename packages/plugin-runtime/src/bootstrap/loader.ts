import { pathToFileURL } from "node:url";

import type { AgentPluginDefinition, ExtensionContext } from "@ora-space/plugin-sdk/agent";

import type { DeclaredAgent } from "../generated/index.js";
import { AGENT_METHODS, type AgentMethodName } from "./contracts.js";

export type AgentHandler = (
  call: Readonly<{ requestId: string; signal: AbortSignal }>,
  request: Readonly<Record<string, unknown>>,
) => unknown;

export interface DispatchProvider {
  readonly id: string;
  readonly contractVersion: 1;
  readonly handlers: Readonly<Record<AgentMethodName, AgentHandler>>;
}

export interface LoadedPlugin {
  readonly definition: AgentPluginDefinition;
  readonly providers: ReadonlyMap<string, DispatchProvider>;
}

export type ModuleImporter = (specifier: string) => Promise<unknown>;

/** Imports the materialized entry only after initialize and validates its structural default ABI. */
export async function loadAndActivatePlugin(
  entryPath: string,
  sessionId: string,
  context: ExtensionContext,
  declaredAgents: readonly DeclaredAgent[],
  importer: ModuleImporter = (specifier) => import(specifier),
): Promise<LoadedPlugin> {
  const specifier = `${pathToFileURL(entryPath).href}?oraSession=${encodeURIComponent(sessionId)}`;
  const module = requireObjectLike(await importer(specifier), "plugin module namespace");
  const definition = validateDefinition(readDataProperty(module, "default"));
  const activation = requireObjectLike(
    await definition.activate(context),
    "Agent activation result",
  );
  const providersValue = readDataProperty(activation, "providers");
  if (!Array.isArray(providersValue)) {
    throw new TypeError("activation providers must be an array");
  }

  const providers = new Map<string, DispatchProvider>();
  for (const provider of providersValue) {
    const snapshot = snapshotProvider(provider);
    if (providers.has(snapshot.id)) {
      throw new TypeError("activation contains a duplicate provider id");
    }
    providers.set(snapshot.id, snapshot);
  }
  const expected = [...declaredAgents]
    .map(({ id, contractVersion }) => `${id}:${contractVersion}`)
    .sort();
  const actual = [...providers.values()]
    .map(({ id, contractVersion }) => `${id}:${contractVersion}`)
    .sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new TypeError("activated providers do not match the manifest declarations");
  }
  return { definition, providers };
}

function validateDefinition(value: unknown): AgentPluginDefinition {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new TypeError("plugin default export must be a plain object");
  }
  const prototype = Object.getPrototypeOf(value) as object | null;
  if (prototype !== Object.prototype && prototype !== null) {
    throw new TypeError("plugin default export must have a plain prototype");
  }
  if (readDataProperty(value, "kind") !== "agent" || readDataProperty(value, "pluginApi") !== 1) {
    throw new TypeError("plugin default export kind/pluginApi mismatch");
  }
  const activate = readDataProperty(value, "activate");
  if (typeof activate !== "function") {
    throw new TypeError("plugin default export must provide activate()");
  }
  const deactivateDescriptor = Object.getOwnPropertyDescriptor(value, "deactivate");
  if (deactivateDescriptor !== undefined && ("get" in deactivateDescriptor || typeof deactivateDescriptor.value !== "function")) {
    throw new TypeError("plugin deactivate must be an own data function");
  }
  return Object.freeze({
    kind: "agent",
    pluginApi: 1,
    activate: activate.bind(value) as AgentPluginDefinition["activate"],
    ...(deactivateDescriptor === undefined
      ? {}
      : { deactivate: (deactivateDescriptor.value as () => void | Promise<void>).bind(value) }),
  });
}

function snapshotProvider(value: unknown): DispatchProvider {
  const provider = requireObjectLike(value, "Agent provider");
  const id = readDataProperty(provider, "id");
  if (typeof id !== "string" || !/^[a-z0-9][a-z0-9._-]*$/u.test(id)) {
    throw new TypeError("provider id is invalid");
  }
  if (readDataProperty(provider, "contractVersion") !== 1) {
    throw new TypeError("provider contractVersion must equal 1");
  }
  const handlers = Object.create(null) as Record<AgentMethodName, AgentHandler>;
  for (const method of AGENT_METHODS) {
    const localName = method.slice("agent.".length);
    handlers[method] = findDataMethod(provider, localName).bind(provider) as AgentHandler;
  }
  return Object.freeze({ id, contractVersion: 1, handlers: Object.freeze(handlers) });
}

function findDataMethod(object: object, name: string): CallableFunction {
  let current: object | null = object;
  while (current !== null) {
    const descriptor = Object.getOwnPropertyDescriptor(current, name);
    if (descriptor !== undefined) {
      if (!("value" in descriptor) || typeof descriptor.value !== "function") {
        throw new TypeError(`provider method ${name} must be a data function`);
      }
      return descriptor.value as CallableFunction;
    }
    current = Object.getPrototypeOf(current) as object | null;
  }
  throw new TypeError(`provider method ${name} is missing`);
}

function readDataProperty(object: object, name: string): unknown {
  const descriptor = Object.getOwnPropertyDescriptor(object, name);
  if (descriptor === undefined || !("value" in descriptor)) {
    throw new TypeError(`${name} must be an own data property`);
  }
  return descriptor.value;
}

function requireObjectLike(value: unknown, field: string): object {
  if ((typeof value !== "object" && typeof value !== "function") || value === null) {
    throw new TypeError(`${field} must be an object`);
  }
  return value;
}
