import type { AgentCli } from "@ora/contracts";

const AGENT_CLI_LABELS: Record<AgentCli, string> = {
  open_code: "OpenCode",
  nga: "NGA",
  code_agent_cli: "CodeAgentCLI",
  claude: "Claude Code",
  codex: "Codex",
};

/** Returns the product-facing name for a stable agent CLI contract value. */
export function agentCliLabel(agentCli: AgentCli): string {
  return AGENT_CLI_LABELS[agentCli];
}
