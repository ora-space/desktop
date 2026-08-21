import { describe, expect, it } from "vitest";
import type { InstalledPlugin } from "@ora/contracts";
import { filterDiscoveredPlugins } from "./filter-discovered-plugins";

const plugins: InstalledPlugin[] = [
  {
    id: "ora.reviewer",
    packageName: "@ora-plugins/reviewer",
    displayName: "Code Reviewer",
    version: "0.1.0",
    kind: "agent",
    main: "dist/index.js",
    agent: { displayName: "Review Agent", contractVersion: 1 },
    enabled: false,
    logo: null,
    runtime: "stopped",
  },
  {
    id: "ora.planner",
    packageName: "@ora-plugins/planner",
    displayName: "Planner",
    version: "0.2.0",
    kind: "agent",
    main: "dist/index.js",
    agent: { displayName: "Plan Agent", contractVersion: 1 },
    enabled: false,
    logo: null,
    runtime: "stopped",
  },
];

describe("filterDiscoveredPlugins", () => {
  it("preserves backend order for an empty query", () => {
    expect(filterDiscoveredPlugins(plugins, "")).toEqual(plugins);
  });

  it.each([
    ["display name", "reviewer", "ora.reviewer"],
    ["package name", "@ora-plugins/planner", "ora.planner"],
    ["Ora id", "ora.reviewer", "ora.reviewer"],
    ["agent display name", "plan agent", "ora.planner"],
  ])("searches by %s", (_field, query, expectedId) => {
    expect(
      filterDiscoveredPlugins(plugins, query).map((plugin) => plugin.id),
    ).toEqual([expectedId]);
  });

  it("returns no dynamic packages when the query does not match", () => {
    expect(filterDiscoveredPlugins(plugins, "missing")).toEqual([]);
  });
});
