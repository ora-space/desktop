/**
 * Identifies one of Ora's bundled agent CLIs by its persisted, namespaced package id.
 *
 * This is the same identity the backend now accepts for any agent — built-in or
 * plugin-provided — so it is not a generated contract type: `@ora/contracts` carries
 * agent identity as a plain `string` because which agents exist depends on installed
 * plugins and is not knowable at build time. `KnownAgentCli` is the closed subset this
 * frontend still special-cases with a fixed label, logo, and picker position; any other
 * identity is a plugin agent the picker does not yet enumerate.
 */
export type KnownAgentCli =
  | "ora-space.opencode"
  | "ora-space.nga"
  | "ora-space.codeagentcli"
  | "ora-space.claude"
  | "ora-space.codex";

/**
 * Human-facing CLI names shown in the model selector and other surfaces.
 * Labels are stable product names, not user-generated data, so they stay
 * hardcoded. Which CLIs exist is known at build time; which models each one
 * offers is not, and comes from the agent's own session configuration.
 */
export const AGENT_CLI_LABELS: Record<KnownAgentCli, string> = {
  "ora-space.opencode": "OpenCode",
  "ora-space.nga": "NGA",
  "ora-space.codeagentcli": "CodeAgentCLI",
  "ora-space.claude": "Claude Code",
  "ora-space.codex": "Codex",
};

/** The order CLIs are offered in, independent of which one is active. */
export const AGENT_CLI_ORDER: KnownAgentCli[] = [
  "ora-space.opencode",
  "ora-space.nga",
  "ora-space.codeagentcli",
  "ora-space.claude",
  "ora-space.codex",
];
