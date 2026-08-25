import { describe, expect, it } from "vitest";
import type { InstalledPlugin } from "@ora/contracts";
import { filterDiscoveredPlugins } from "./filter-discovered-plugins";

const plugins: InstalledPlugin[] = [
  {
    id: "official/ora.reviewer",
    namespace: "official",
    name: "ora.reviewer",
    description: "ora.reviewer plugin",
    homepage: null,
    license: null,
    displayName: "Code Reviewer",
    version: "0.1.0",
    kind: "agent",
    agentDisplayName: "Review Agent",
    enabled: false,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "stopped",
  },
  {
    id: "official/ora.planner",
    namespace: "official",
    name: "ora.planner",
    description: "ora.planner plugin",
    homepage: null,
    license: null,
    displayName: "Planner",
    version: "0.2.0",
    kind: "agent",
    agentDisplayName: "Plan Agent",
    enabled: false,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "stopped",
  },
  {
    id: "official/skill-test",
    namespace: "official",
    name: "skill-test",
    description: "Static skill package",
    homepage: null,
    license: null,
    displayName: "skill-test",
    version: "0.1.0",
    kind: "skill",
    enabled: true,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "stopped",
  },
];

describe("filterDiscoveredPlugins", () => {
  it("preserves backend order for an empty query", () => {
    expect(filterDiscoveredPlugins(plugins, "")).toEqual(plugins);
  });

  it.each([
    ["display name", "reviewer", "official/ora.reviewer"],
    ["description", "ora.planner plugin", "official/ora.planner"],
    ["canonical id", "official/ora.reviewer", "official/ora.reviewer"],
    ["agent display name", "plan agent", "official/ora.planner"],
    ["skill description", "static skill", "official/skill-test"],
  ])("searches by %s", (_field, query, expectedId) => {
    expect(
      filterDiscoveredPlugins(plugins, query).map((plugin) => plugin.id),
    ).toEqual([expectedId]);
  });

  it("returns no dynamic packages when the query does not match", () => {
    expect(filterDiscoveredPlugins(plugins, "missing")).toEqual([]);
  });
});
