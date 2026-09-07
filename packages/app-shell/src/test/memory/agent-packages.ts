import { type InstalledPlugin } from "@ora/contracts";
import {
  AGENT_PACKAGES,
  SEEDED_NAMESPACE,
  officialAgentRef,
} from "../agent-identity";

/** Builds the installed-package record one seeded agent is supplied by. */
function agentPackage(name: string, displayName: string): InstalledPlugin {
  return {
    id: officialAgentRef(name),
    namespace: SEEDED_NAMESPACE,
    name,
    displayName,
    version: "1.0.0",
    description: `${displayName} agent`,
    homepage: null,
    license: null,
    kind: "agent",
    agentDisplayName: displayName,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "running",
  };
}

/** Builds the explicit official agent package fixture used by runtime/plugin tests. */
export function seededAgentPackages(): InstalledPlugin[] {
  return AGENT_PACKAGES.map((agent) =>
    agentPackage(agent.name, agent.displayName),
  );
}
