import type * as acp from "@agentclientprotocol/sdk";
import type { TokenUsageReport } from "@ora/contracts";
import type { SessionUsage } from "./types.ts";

/** Creates the usage state for a new conversation that has no history to explain. */
export function createHiddenSessionUsage(): SessionUsage {
  return {
    context: { status: "hidden" },
    lastTurnTokens: { status: "none" },
  };
}

/** Creates the empty telemetry state shown after a durable transcript is reloaded. */
export function createReloadedSessionUsage(hasHistory: boolean): SessionUsage {
  return {
    context: hasHistory
      ? { status: "needs_interaction" }
      : { status: "hidden" },
    lastTurnTokens: { status: "none" },
  };
}

/** Starts one prompt without discarding a still-valid context snapshot. */
export function beginUsageTurn(usage: SessionUsage): SessionUsage {
  return {
    context:
      usage.context.status === "reported"
        ? usage.context
        : { status: "awaiting_report" },
    lastTurnTokens: { status: "awaiting_completion" },
  };
}

/** Clears telemetry when the conversation is handed to a different agent. */
export function beginUsageAfterAgentSwitch(): SessionUsage {
  return {
    context: { status: "awaiting_report" },
    lastTurnTokens: { status: "awaiting_completion" },
  };
}

/** Replaces the context snapshot with the latest stable ACP usage update. */
export function applyContextUsageUpdate(
  usage: SessionUsage,
  update: acp.UsageUpdate,
  receivedAt: number,
): SessionUsage {
  return {
    ...usage,
    context: {
      status: "reported",
      snapshot: {
        usedTokens: update.used,
        sizeTokens: update.size,
        ...(update.cost === undefined || update.cost === null
          ? {}
          : { cost: update.cost }),
        receivedAt,
      },
    },
  };
}

/** Settles the usage state from one completed prompt response. */
export function completeUsageTurn(
  usage: SessionUsage,
  tokenUsage: TokenUsageReport | undefined,
  receivedAt: number,
): SessionUsage {
  return {
    context:
      usage.context.status === "reported"
        ? usage.context
        : { status: "unavailable" },
    lastTurnTokens:
      tokenUsage === undefined
        ? { status: "unavailable" }
        : { status: "reported", usage: tokenUsage, receivedAt },
  };
}
