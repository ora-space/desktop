import { PluginMethodError } from "../plugin.ts";
import { AGENT_NOT_INSTALLED } from "../agent/contract.ts";

/** Classifies a spawn failure as a missing binary, tolerating platform error wording. */
export function isCommandNotFound(error: unknown): boolean {
  if (error instanceof Deno.errors.NotFound) {
    return true;
  }
  const message = error instanceof Error ? error.message : String(error);
  return /not found|cannot find|no such file|is not recognized/i.test(message);
}

/**
 * Runs `attempt` against every candidate command until one does not throw.
 *
 * Failures are classified on the way out: a failure that is not "binary missing" is the real
 * startup fault and is rethrown as-is, while an exhausted candidate list means the external
 * program is simply absent. That distinction is the whole point — Ora retries
 * `AGENT_NOT_INSTALLED` quietly as expected local configuration, and logs anything else as a
 * fault. `notInstalledMessage` receives the tried candidates so the plugin can name its own
 * install instructions.
 */
export async function tryEachCandidate<T>(
  candidates: readonly string[],
  attempt: (command: string) => T | Promise<T>,
  notInstalledMessage: (tried: readonly string[]) => string,
): Promise<T> {
  if (candidates.length === 0) {
    throw new Error("tryEachCandidate needs at least one command candidate");
  }
  const failures: unknown[] = [];
  for (const command of candidates) {
    try {
      return await attempt(command);
    } catch (error) {
      failures.push(error);
    }
  }

  const realFailure = failures.find((error) => !isCommandNotFound(error));
  if (realFailure !== undefined) {
    throw realFailure instanceof Error
      ? realFailure
      : new Error(String(realFailure));
  }
  throw new PluginMethodError(
    AGENT_NOT_INSTALLED,
    notInstalledMessage(candidates),
  );
}

/** Reads an env var, treating a missing read permission as an unset value. */
export function readEnv(name: string): string | undefined {
  try {
    return Deno.env.get(name);
  } catch {
    // The host may not grant --allow-env; absence is indistinguishable from "not set".
    return undefined;
  }
}

/**
 * Expands a bare command into the spellings a Windows npm or standalone install exposes.
 *
 * npm installs only a `<name>.cmd` shim while scoop, bun, or a standalone binary expose the bare
 * name; trying both keeps either installation style working with no user configuration. On other
 * platforms the bare name is the only candidate.
 */
export function platformCommandCandidates(name: string): string[] {
  return Deno.build.os === "windows" ? [`${name}.cmd`, name] : [name];
}
