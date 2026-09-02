import {
  createPlugin,
  type EffectResourceDeclaration,
  type Plugin,
  PluginMethodError,
} from "./plugin.ts";
import {
  AGENT_METHODS,
  AGENT_NOT_INSTALLED,
  AGENT_UNUSABLE,
  type AgentEffectCoordinationContext,
  type AgentEffectReadinessContext,
  type AgentListModelsResult,
  type AgentListModelsParams,
  type AgentModel as WireAgentModel,
  type AgentStartContext,
  type AgentStartResult,
  EFFECT_METHODS,
  INVALID_PARAMS,
  type JsonValue,
  SUPPORTED_ACP_VERSION,
} from "./protocol/index.ts";

export type {
  AgentEffectCoordinationContext,
  AgentEffectReadinessContext,
  AgentStartContext,
};

/**
 * The error code that tells Ora the agent CLI is absent from this machine.
 *
 * Ora treats it as an expected local configuration: the connection retries quietly instead of
 * reporting a fault, so use it only when the agent genuinely is not installed.
 */
export { AGENT_NOT_INSTALLED };

/**
 * The error code that tells Ora the agent this package ships cannot run on this machine.
 *
 * Unlike {@link AGENT_NOT_INSTALLED}, this is not something the user can fix while Ora runs: the
 * same package fails the same way on every attempt, so Ora reports it once and stops retrying that
 * agent. Use it when the executable this package carries is broken, missing its dependencies, or
 * built for another target — never when a CLI is merely absent from PATH.
 */
export { AGENT_UNUSABLE };

/** Describes one model the agent offers before any session exists. */
export type AgentModel = Omit<WireAgentModel, "default"> & {
  default?: boolean;
};

/** Sends one ACP frame from the agent back to the host. */
export type AcpSender = (frame: JsonValue) => Promise<void>;

/** Defines Agent-consumed Resources and its coordination/readiness adapter methods. */
export interface AgentEffectDefinition {
  resources: readonly EffectResourceDeclaration[];
  coordinate(
    context: AgentEffectCoordinationContext,
  ): JsonValue | Promise<JsonValue>;
  reactivate(
    context: AgentEffectCoordinationContext,
  ): JsonValue | Promise<JsonValue>;
  verifyReady(
    context: AgentEffectReadinessContext,
  ): JsonValue | Promise<JsonValue>;
}

/** Implements the agent contract Ora requires of every `kind: "agent"` plugin. */
export interface AgentDefinition {
  /**
   * Brings the agent up so it can receive ACP frames.
   *
   * Throw `new PluginMethodError(AGENT_NOT_INSTALLED, ...)` when the underlying CLI is missing,
   * and `AGENT_UNUSABLE` when the one this package ships cannot run at all; `spawnAgentProcess`
   * raises both for a plugin that resolves its CLI through the host.
   */
  start(context: AgentStartContext, send: AcpSender): void | Promise<void>;
  /** Terminates the agent while leaving this plugin process alive. */
  stop(): void | Promise<void>;
  /** Lists selectable models outside any session. */
  listModels(context: { cwd: string }): AgentModel[] | Promise<AgentModel[]>;
  /** Receives one ACP frame the host is forwarding to the agent. */
  onAcp(frame: JsonValue): void | Promise<void>;
  /** Declares Resources this Agent consumes and the adapter proving safe convergence. */
  effects?: AgentEffectDefinition;
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
  const send: AcpSender = (frame) => plugin.notify(AGENT_METHODS.acp, frame);

  plugin.declareEmit(AGENT_METHODS.acp);
  plugin.registerMethod(AGENT_METHODS.start, async (input) => {
    await definition.start(parseStartContext(input), send);
    // ACP is the only protocol Ora carries today; the field exists so a plugin that translates a
    // private protocol can declare it later without changing the notification channel.
    return {
      protocol: "acp",
      acpVersion: SUPPORTED_ACP_VERSION,
    } satisfies AgentStartResult;
  });
  plugin.registerMethod(AGENT_METHODS.stop, async () => {
    await definition.stop();
    return {};
  });
  plugin.registerMethod(AGENT_METHODS.listModels, async (params) =>
    ({
      models: (
        await definition.listModels(params as AgentListModelsParams)
      ).map((model) => ({
        id: model.id,
        displayName: model.displayName,
        default: model.default ?? false,
      })),
    }) satisfies AgentListModelsResult);
  plugin.onNotification(
    AGENT_METHODS.acp,
    (params) => definition.onAcp(params),
  );
  const effects = definition.effects;
  if (effects !== undefined) {
    for (const resource of effects.resources) {
      plugin.declareEffectResource(resource);
    }
    plugin.registerMethod(
      EFFECT_METHODS.coordinate,
      (input) => effects.coordinate(parseCoordinationContext(input)),
    );
    plugin.registerMethod(
      EFFECT_METHODS.reactivate,
      (input) => effects.reactivate(parseCoordinationContext(input)),
    );
    plugin.registerMethod(
      EFFECT_METHODS.verifyReady,
      (input) => effects.verifyReady(parseReadinessContext(input)),
    );
  }

  return plugin;
}

/** Validates the exact generic identities used by Consumer coordination. */
function parseCoordinationContext(
  input: JsonValue,
): AgentEffectCoordinationContext {
  if (
    typeof input !== "object" || input === null || Array.isArray(input) ||
    typeof input.targetId !== "string" ||
    !Array.isArray(input.resourceIds) ||
    !input.resourceIds.every((resource) => typeof resource === "string")
  ) {
    throw new PluginMethodError(
      INVALID_PARAMS,
      "Effect coordination requires targetId and resourceIds",
    );
  }
  return {
    targetId: input.targetId,
    resourceIds: input.resourceIds as string[],
  };
}

/** Validates exact Consumer Revision and projection identity before readiness logic runs. */
function parseReadinessContext(input: JsonValue): AgentEffectReadinessContext {
  if (
    typeof input !== "object" || input === null || Array.isArray(input) ||
    typeof input.targetId !== "string" ||
    typeof input.generation !== "number" ||
    !Number.isSafeInteger(input.generation) || input.generation < 0 ||
    typeof input.consumerRevisionId !== "string" ||
    typeof input.projectionDigest !== "string"
  ) {
    throw new PluginMethodError(
      INVALID_PARAMS,
      `${EFFECT_METHODS.verifyReady} requires exact Target projection identity`,
    );
  }
  return {
    targetId: input.targetId,
    generation: input.generation,
    consumerRevisionId: input.consumerRevisionId,
    projectionDigest: input.projectionDigest,
  };
}

/** Validates the host's start parameters before the agent implementation sees them. */
function parseStartContext(input: JsonValue): AgentStartContext {
  if (
    typeof input !== "object" || input === null || Array.isArray(input) ||
    typeof input.cwd !== "string" || typeof input.hostVersion !== "string"
  ) {
    throw new PluginMethodError(
      INVALID_PARAMS,
      "agent/start requires a cwd and hostVersion",
    );
  }
  return { cwd: input.cwd, hostVersion: input.hostVersion };
}
