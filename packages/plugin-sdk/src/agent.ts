import { createPlugin, type Plugin, PluginMethodError } from "./plugin.ts";
import type { JsonValue } from "./protocol.ts";

const AGENT_START = "agent/start";
const AGENT_STOP = "agent/stop";
const AGENT_LIST_MODELS = "agent/listModels";
const AGENT_ACP = "agent/acp";

/**
 * The error code that tells Ora the agent CLI is absent from this machine.
 *
 * Ora treats it as an expected local configuration: the connection retries quietly instead of
 * reporting a fault, so use it only when the agent genuinely is not installed.
 */
export const AGENT_NOT_INSTALLED = -32001;

/** Describes one model the agent offers before any session exists. */
export interface AgentModel {
  id: string;
  displayName: string;
  default?: boolean;
}

/** Carries the host context handed to an agent when it starts. */
export interface AgentStartContext {
  /** Neutral working directory the agent should start in. */
  cwd: string;
  /** Version of the Ora host that launched this plugin. */
  hostVersion: string;
}

/** Sends one ACP frame from the agent back to the host. */
export type AcpSender = (frame: JsonValue) => Promise<void>;

/** Implements the agent contract Ora requires of every `kind: "agent"` plugin. */
export interface AgentDefinition {
  /**
   * Brings the agent up so it can receive ACP frames.
   *
   * Throw `new PluginMethodError(AGENT_NOT_INSTALLED, ...)` when the underlying CLI is missing.
   */
  start(context: AgentStartContext, send: AcpSender): void | Promise<void>;
  /** Terminates the agent while leaving this plugin process alive. */
  stop(): void | Promise<void>;
  /** Lists selectable models outside any session. */
  listModels(): AgentModel[] | Promise<AgentModel[]>;
  /** Receives one ACP frame the host is forwarding to the agent. */
  onAcp(frame: JsonValue): void | Promise<void>;
}

/**
 * Builds a plugin that serves Ora's agent contract.
 *
 * The whole contract is registered up front — the three control methods plus the `agent/acp`
 * notification in both directions — because Ora validates it the moment the handshake completes
 * and refuses to use a plugin whose declaration is incomplete.
 */
export function defineAgent(definition: AgentDefinition): Plugin {
  const plugin = createPlugin();
  const send: AcpSender = (frame) => plugin.notify(AGENT_ACP, frame);

  plugin.declareEmit(AGENT_ACP);
  plugin.registerMethod(AGENT_START, async (input) => {
    await definition.start(parseStartContext(input), send);
    // ACP is the only protocol Ora carries today; the field exists so a plugin that translates a
    // private protocol can declare it later without changing the notification channel.
    return { protocol: "acp", acpVersion: 1 };
  });
  plugin.registerMethod(AGENT_STOP, async () => {
    await definition.stop();
    return {};
  });
  plugin.registerMethod(AGENT_LIST_MODELS, async () => ({
    models: (await definition.listModels()).map((model) => ({
      id: model.id,
      displayName: model.displayName,
      default: model.default ?? false,
    })),
  }));
  plugin.onNotification(AGENT_ACP, (params) => definition.onAcp(params));

  return plugin;
}

/** Validates the host's start parameters before the agent implementation sees them. */
function parseStartContext(input: JsonValue): AgentStartContext {
  if (
    typeof input !== "object" || input === null || Array.isArray(input) ||
    typeof input.cwd !== "string" || typeof input.hostVersion !== "string"
  ) {
    throw new PluginMethodError(
      -32602,
      "agent/start requires a cwd and hostVersion",
    );
  }
  return { cwd: input.cwd, hostVersion: input.hostVersion };
}
