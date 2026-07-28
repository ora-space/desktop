import type { AgentCli } from "@ora/contracts";
import { useAgentModels } from "../../state/hooks/use-agent-models";

/**
 * Human-facing CLI names shown in the model selector and other surfaces.
 * Labels are stable product names, not user-generated data, so they stay
 * hardcoded. The model lists themselves come from the `listAgentModels` API.
 */
export const AGENT_CLI_LABELS: Record<AgentCli, string> = {
  open_code: "OpenCode",
  nga: "NGA",
  code_agent_cli: "CodeAgentCLI",
  claude: "Claude Code",
  codex: "Codex",
};

/**
 * The order used when listing every CLI's models in the dropdown.
 * Derived from a stable preference, with the active CLI moved to the front
 * so it is always immediately reachable.
 */
export function orderedGroups(
  groups: Array<{ agentCli: AgentCli; models: Array<string> }>,
  activeCli: AgentCli,
) {
  const preferred: AgentCli[] = ["open_code", "nga", "code_agent_cli", "claude", "codex"];
  const sorted = [...groups].sort(
    (a, b) => preferred.indexOf(a.agentCli) - preferred.indexOf(b.agentCli),
  );
  if (sorted.findIndex((g) => g.agentCli === activeCli) > 0) {
    const idx = sorted.findIndex((g) => g.agentCli === activeCli);
    if (idx > 0) {
      const [moved] = sorted.splice(idx, 1);
      sorted.unshift(moved!);
    }
  }
  return sorted;
}

/**
 * Convenience hook that delegates to `useAgentModels` so the selector doesn't
 * need to reach into the hooks directory directly.
 */
export function useAvailableModels() {
  return useAgentModels();
}
