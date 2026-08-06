import type { AgentCli } from "@ora/contracts";

/**
 * Human-facing CLI names shown in the model selector and other surfaces.
 * Labels are stable product names, not user-generated data, so they stay
 * hardcoded. Which CLIs exist is known at build time; which models each one
 * offers is not, and comes from the agent's own session configuration.
 */
export const AGENT_CLI_LABELS: Record<AgentCli, string> = {
  open_code: "OpenCode",
  nga: "NGA",
  code_agent_cli: "CodeAgentCLI",
  claude: "Claude Code",
  codex: "Codex",
};

/** The order CLIs are offered in, independent of which one is active. */
export const AGENT_CLI_ORDER: AgentCli[] = [
  "open_code",
  "nga",
  "code_agent_cli",
  "claude",
  "codex",
];
