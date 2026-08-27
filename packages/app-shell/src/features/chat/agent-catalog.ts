import { useMemo } from "react";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";

/**
 * Describes one agent this installation can bind a session to.
 *
 * Every field comes from the package that supplies the agent. Nothing about which agents exist,
 * what they are called, or how they are drawn is known at build time: an agent arrives with an
 * installed plugin and leaves when that plugin is uninstalled.
 */
export interface AgentEntry {
  /**
   * The agent's persisted, namespaced identity — the package name, which is exactly what a
   * session stores as its `agentRef` and what the backend keys its runtime by.
   */
  agentRef: string;
  /** Name shown in the pickers, declared by the package's agent contribution. */
  label: string;
  /** Inline SVG source for the package's brand mark, absent when it ships none. */
  logo: string | null;
}

/**
 * Lists every agent supplied by an installed package, in the backend's stable identifier order.
 *
 * This is the catalog, not the offer: a package that is installed but disabled, or whose agent is
 * not runnable on this machine, still appears here so a session bound to it can be labelled. Use
 * `useAvailableAgents` for the set a picker may offer.
 */
export function useAgentCatalog(): AgentEntry[] {
  const { data: plugins } = useInstalledPlugins();
  return useMemo(
    () =>
      (plugins ?? [])
        .filter((plugin) => plugin.kind === "agent")
        .map((plugin) => ({
          // Supervisors are keyed by the package name rather than its `namespace/name` address,
          // because the name alone is what every session persisted as its binding.
          agentRef: plugin.name,
          label: plugin.agentDisplayName,
          logo: plugin.logo,
        })),
    [plugins],
  );
}

/**
 * Resolves how one bound agent should be named and drawn.
 *
 * A session can outlive the package that supplied its agent, so a miss is an ordinary state
 * rather than an error. Callers render the raw identity in that case, which is the only honest
 * thing left to show.
 */
export function useAgentEntry(agentRef: string | null): AgentEntry | undefined {
  const catalog = useAgentCatalog();
  return catalog.find((entry) => entry.agentRef === agentRef);
}

/** Names one agent for display, falling back to its raw identity when nothing supplies it. */
export function agentLabel(
  catalog: readonly AgentEntry[],
  agentRef: string,
): string {
  return (
    catalog.find((entry) => entry.agentRef === agentRef)?.label ?? agentRef
  );
}
