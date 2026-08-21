import { describe, expect, it } from "vitest";
import type { InstalledPlugin } from "@ora/contracts";
import { listSurfaceDefinitions } from "./surface-definitions";

function uiPlugin(
  id: string,
  displayName: string,
  enabled: boolean,
  surfaces: Array<
    | { id: string; title: string; entryUrl: string }
    | { id: string; title: string; source: "panel" }
  >,
): InstalledPlugin {
  return {
    id,
    packageName: `@ora-space/${id}`,
    displayName,
    version: "0.1.0",
    main: "dist/index.js",
    kind: "ui",
    contractVersion: 1,
    surfaces: surfaces.map((surface) =>
      "source" in surface
        ? surface
        : { ...surface, source: "remote_site" as const },
    ),
    enabled,
    runtime: "stopped",
  };
}

const agentPlugin: InstalledPlugin = {
  id: "ora.reviewer",
  packageName: "@ora-plugins/reviewer",
  displayName: "Code Reviewer",
  version: "0.1.0",
  main: "dist/index.js",
  kind: "agent",
  agentDisplayName: "Review Agent",
  contractVersion: 1,
  enabled: true,
  runtime: "running",
};

describe("listSurfaceDefinitions", () => {
  it("returns only enabled ui plugin surfaces sorted by plugin name then title", () => {
    const plugins = [
      agentPlugin,
      uiPlugin("ora-space.skillhub", "SkillHub", true, [
        { id: "market", title: "Market", entryUrl: "https://www.skillhub.cn/" },
        { id: "docs", title: "Docs", entryUrl: "https://docs.skillhub.cn/" },
      ]),
      uiPlugin("ora-space.disabled", "Disabled", false, [
        { id: "x", title: "X", entryUrl: "https://example.com/" },
      ]),
      uiPlugin("ora-space.huawei", "Huawei", true, [
        {
          id: "dev",
          title: "Developer",
          entryUrl: "https://developer.huawei.com/",
        },
      ]),
      uiPlugin("ora-space.hello-panel", "Hello Panel", true, [
        { id: "counter", title: "Counter", source: "panel" },
      ]),
    ];

    expect(listSurfaceDefinitions(plugins)).toEqual([
      {
        pluginId: "ora-space.hello-panel",
        surfaceId: "counter",
        title: "Counter",
        pluginDisplayName: "Hello Panel",
      },
      {
        pluginId: "ora-space.huawei",
        surfaceId: "dev",
        title: "Developer",
        pluginDisplayName: "Huawei",
      },
      {
        pluginId: "ora-space.skillhub",
        surfaceId: "docs",
        title: "Docs",
        pluginDisplayName: "SkillHub",
      },
      {
        pluginId: "ora-space.skillhub",
        surfaceId: "market",
        title: "Market",
        pluginDisplayName: "SkillHub",
      },
    ]);
  });

  it("returns an empty list when no ui plugin is enabled", () => {
    expect(listSurfaceDefinitions([agentPlugin])).toEqual([]);
  });
});
