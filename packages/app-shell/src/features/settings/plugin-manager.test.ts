import { describe, expect, it } from "vitest";
import type { InstalledPlugin } from "@ora/contracts";
import {
  filterDiscoveredPlugins,
  filterInstalledPlugins,
} from "./filter-discovered-plugins";

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

describe("filterInstalledPlugins", () => {
  /** Two installed packages whose canonical ids differ only by a slug suffix. */
  const siblings: InstalledPlugin[] = [
    { ...plugins[0]!, id: "official/ora-space.review" },
    { ...plugins[0]!, id: "official/ora-space.review-v2" },
  ];

  it("locates exactly the named plugin when a sibling id extends it as a prefix", () => {
    expect(
      filterInstalledPlugins(siblings, {
        kind: "locate",
        pluginId: "official/ora-space.review",
      }).map((plugin) => plugin.id),
    ).toEqual(["official/ora-space.review"]);
  });

  it("keeps fuzzy search for typed text, so the prefix sibling matches too", () => {
    expect(
      filterInstalledPlugins(siblings, {
        kind: "search",
        text: "official/ora-space.review",
      }).map((plugin) => plugin.id),
    ).toEqual(["official/ora-space.review", "official/ora-space.review-v2"]);
  });

  it("matches a located id case-insensitively, like search matches ids", () => {
    expect(
      filterInstalledPlugins(plugins, {
        kind: "locate",
        pluginId: "OFFICIAL/ORA.REVIEWER",
      }).map((plugin) => plugin.id),
    ).toEqual(["official/ora.reviewer"]);
  });

  it("returns nothing when the located plugin is not installed", () => {
    expect(
      filterInstalledPlugins(plugins, {
        kind: "locate",
        pluginId: "official/missing",
      }),
    ).toEqual([]);
  });

  it("returns every plugin for an empty typed search", () => {
    expect(
      filterInstalledPlugins(plugins, { kind: "search", text: "" }),
    ).toEqual(plugins);
  });
});
