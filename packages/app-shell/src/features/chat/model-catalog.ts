import type { AgentCli, acp } from "@ora/contracts";

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
};

/** The order CLIs are offered in, independent of which one is active. */
export const AGENT_CLI_ORDER: AgentCli[] = ["open_code", "nga", "code_agent_cli"];

/**
 * Finds the agent's model selector among its configuration options.
 *
 * `category` is a UX hint the protocol says clients must tolerate missing, so a
 * lone select option is treated as the model picker when nothing is categorised.
 * An agent that exposes no selectable model yields `null` and the picker shows
 * its empty state rather than inventing choices.
 */
export function findModelOption(
  configOptions: acp.SessionConfigOption[],
): acp.SessionConfigOption | null {
  const selects = configOptions.filter((option) => option.type === "select");
  return (
    selects.find((option) => option.category === "model")
    ?? (selects.length === 1 ? selects[0]! : null)
  );
}

/** Flattens grouped and ungrouped select values into one ordered list. */
export function selectableValues(
  option: acp.SessionConfigOption,
): acp.SessionConfigSelectOption[] {
  if (option.type !== "select") return [];
  return option.options.flatMap((entry) => ("group" in entry ? entry.options : [entry]));
}

/** Returns the human-readable name of the option value currently in effect. */
export function currentValueName(option: acp.SessionConfigOption): string | null {
  if (option.type !== "select") return null;
  const current = selectableValues(option).find(
    (value) => value.value === option.currentValue,
  );
  return current?.name ?? option.currentValue;
}
