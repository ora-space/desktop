import { AGENT_COMMANDS_WAIT_CAP_MS } from "./protocol/index.ts";

/** How much of a discovery's budget {@link waitForAgentCommands} leaves for cleanup by default. */
export const AGENT_DISCOVERY_CLEANUP_RESERVE_MS = 5_000;

/** Bounds one wait for an agent's first command report. */
export interface AgentCommandsWait {
  /** Waits less than Ora's cap. A larger value has no effect. */
  limitMs?: number;
  /** When the whole discovery must be over, in `Date.now()` time. */
  deadline?: number;
  /** Time kept before `deadline` for removing the probe session and ending its process. */
  cleanupReserveMs?: number;
}

/**
 * Waits for the first command report of a discovery's probe session.
 *
 * Call it once `session/new` has answered, with a promise for the first `available_commands_update`
 * of that session, which may already have settled. It resolves to `undefined` — commands unknown —
 * when nothing arrives in time or the report fails, so a missing report never costs the models
 * already read. The wait ends at Ora's cap or where the cleanup reserve begins, whichever is first.
 */
export async function waitForAgentCommands<Report>(
  report: Promise<Report>,
  wait: AgentCommandsWait = {},
): Promise<Report | undefined> {
  const limits = [AGENT_COMMANDS_WAIT_CAP_MS];
  if (wait.limitMs !== undefined) limits.push(wait.limitMs);
  if (wait.deadline !== undefined) {
    const reserve = wait.cleanupReserveMs ?? AGENT_DISCOVERY_CLEANUP_RESERVE_MS;
    limits.push(wait.deadline - reserve - Date.now());
  }
  let timer: ReturnType<typeof setTimeout> | undefined;
  const expired = new Promise<undefined>((resolve) => {
    timer = setTimeout(resolve, Math.max(0, Math.min(...limits)), undefined);
  });
  try {
    return await Promise.race([report.catch(() => undefined), expired]);
  } finally {
    clearTimeout(timer);
  }
}
