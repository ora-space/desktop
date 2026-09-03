/**
 * Names the agents the test fixtures seed, in the spelling the backend actually reports.
 *
 * This module exists so exactly one place decides what an agent identity looks like. An agent is
 * identified by its package's whole `namespace/name` id, because two marketplace sources may
 * publish the same name and the name alone would collapse them into a single agent. The frontend
 * once keyed its catalog by the bare name while the runtime keyed its supervisors by the id, and
 * every fixture that spelled the bare name agreed with the bug rather than catching it — so tests
 * take the identity from here instead of joining the halves themselves.
 */

/** Namespace the reserved first-party marketplace installs its packages under. */
const OFFICIAL_NAMESPACE = "official";

/** Builds the agent identity one first-party package supplies. */
export function officialAgentRef(name: string): string {
  return `${OFFICIAL_NAMESPACE}/${name}`;
}

/**
 * Every agent this mock installation offers, supplied by an installed package and detected.
 *
 * Agents exist only because a package supplies them, so a test needs both halves to see one in a
 * picker: the installed package that names it, and a runtime status that reaches it. A test that
 * needs one unreachable overrides `agentRuntimeStatuses`; one that needs it gone entirely
 * overrides `installedPlugins` as well.
 */
export const AGENT_PACKAGES = [
  { key: "opencode", name: "ora-space.opencode", displayName: "OpenCode" },
  { key: "nga", name: "ora-space.nga", displayName: "NGA" },
  {
    key: "codeagentcli",
    name: "ora-space.codeagentcli",
    displayName: "CodeAgentCLI",
  },
  { key: "claude", name: "ora-space.claude", displayName: "Claude Code" },
  { key: "codex", name: "ora-space.codex", displayName: "Codex" },
] as const;

/** The namespace every seeded package is installed under. */
export const SEEDED_NAMESPACE = OFFICIAL_NAMESPACE;

/** The agent identity each seeded package supplies, by short key. */
export const AGENT_REF = Object.fromEntries(
  AGENT_PACKAGES.map((agent) => [agent.key, officialAgentRef(agent.name)]),
) as Record<(typeof AGENT_PACKAGES)[number]["key"], string>;

/** Every seeded agent identity, in the order the packages are listed. */
export const SEEDED_AGENT_REFS = AGENT_PACKAGES.map((agent) =>
  officialAgentRef(agent.name),
);
