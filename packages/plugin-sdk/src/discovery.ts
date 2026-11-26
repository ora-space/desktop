import type { AgentDiscoveryContext } from "./agent.ts";
import {
  AGENT_COMMANDS_WAIT_CAP_MS,
  type AgentDiscoverSkills,
  type JsonValue,
  SKILL_DIRECTORIES_META_KEY,
} from "./protocol/index.ts";

/** How much of a discovery's time {@link waitForAgentCommands} leaves for cleanup by default. */
export const AGENT_DISCOVERY_CLEANUP_RESERVE_MS = 5_000;

/** Shortens one wait for an agent's first command report. Nothing here can lengthen it. */
export interface AgentCommandsWait {
  /** Waits less than Ora's cap. */
  limitMs?: number;
  /** A deadline of the plugin's own, in `Date.now()` time, when it comes before Ora's. */
  deadline?: number;
  /** Time kept for removing the probe session and ending its process. */
  cleanupReserveMs?: number;
}

/**
 * Waits for the first command report of a discovery's probe session.
 *
 * Call it once `session/new` has answered. `context` is the one `discover` received, whose deadline
 * is Ora's; `report` is a promise for the first `available_commands_update` of that session, which
 * may already have settled. It resolves to `undefined` — commands unknown — when nothing arrives in time or the report fails,
 * so a missing report never costs the models already read. The wait ends at Ora's cap or where the
 * cleanup reserve before the deadline begins, whichever is first.
 */
export async function waitForAgentCommands<Report>(
  context: Pick<AgentDiscoveryContext, "deadline">,
  report: Promise<Report>,
  wait: AgentCommandsWait = {},
): Promise<Report | undefined> {
  const deadline = Math.min(context.deadline, wait.deadline ?? Infinity);
  const reserve = wait.cleanupReserveMs ?? AGENT_DISCOVERY_CLEANUP_RESERVE_MS;
  const limit = Math.min(
    AGENT_COMMANDS_WAIT_CAP_MS,
    wait.limitMs ?? Infinity,
    deadline - reserve - Date.now(),
  );
  let timer: ReturnType<typeof setTimeout> | undefined;
  const expired = new Promise<undefined>((resolve) => {
    timer = setTimeout(resolve, Math.max(0, limit), undefined);
  });
  try {
    return await Promise.race([report.catch(() => undefined), expired]);
  } finally {
    clearTimeout(timer);
  }
}

/** The part of an ACP session setup that hands an agent its Skills. */
export interface SkillSessionSetup {
  additionalDirectories?: string[];
  _meta?: Record<string, JsonValue>;
}

/**
 * Builds the Skill part of a probe session's `session/new`, as Ora builds it for a session.
 *
 * Spread it into the params. `supportsAdditionalDirectories` is whether the agent's `initialize`
 * answer declared `sessionCapabilities.additionalDirectories`; without it, or without Skills, the
 * fields an agent does not expect stay out.
 */
export function skillSessionSetup(
  skills: AgentDiscoverSkills | undefined,
  supportsAdditionalDirectories: boolean,
): SkillSessionSetup {
  if (skills === undefined) return {};
  const { readRoots, directories } = skills;
  return {
    ...(supportsAdditionalDirectories && readRoots.length > 0
      ? { additionalDirectories: [...readRoots] }
      : {}),
    ...(directories.length > 0
      ? { _meta: { [SKILL_DIRECTORIES_META_KEY]: [...directories] } }
      : {}),
  };
}
