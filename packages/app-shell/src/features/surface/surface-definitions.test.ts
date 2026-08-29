import { describe, expect, it } from "vitest";
import type { InstalledPlugin } from "@ora/contracts";
import { listSurfaceDefinitions } from "./surface-definitions";

/** Builds one installed webview plugin (external site). */
function webviewPlugin(
  id: string,
  displayName: string,
  title: string,
): InstalledPlugin {
  return {
    id: `official/${id}`,
    namespace: "official",
    name: id,
    displayName,
    description: `${displayName} plugin`,
    homepage: null,
    license: null,
    version: "0.1.0",
    kind: "webview",
    title,
    startUrl: "https://www.example.com/",
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "stopped",
  };
}

/** Builds one workbench plugin (package-shipped page). */
function workbenchPlugin(
  id: string,
  displayName: string,
  title: string,
): InstalledPlugin {
  return {
    id: `official/${id}`,
    namespace: "official",
    name: id,
    displayName,
    description: `${displayName} plugin`,
    homepage: null,
    license: null,
    version: "0.1.0",
    kind: "workbench",
    title,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "stopped",
  };
}

const agentPlugin: InstalledPlugin = {
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
  runtime: "running",
};

describe("listSurfaceDefinitions", () => {
  it("returns one entry per installed workbench or webview plugin sorted by name then title", () => {
    const plugins = [
      agentPlugin,
      webviewPlugin("acme.hub", "Hub", "Market"),
      webviewPlugin("ora-space.tools", "Tools", "X"),
      webviewPlugin("acme.portal", "Portal", "Developer"),
      workbenchPlugin("acme.panel", "Panel", "Counter"),
    ];

    expect(listSurfaceDefinitions(plugins)).toEqual([
      {
        pluginId: "official/acme.hub",
        kind: "webview",
        title: "Market",
        pluginDisplayName: "Hub",
        logo: null,
      },
      {
        pluginId: "official/acme.panel",
        kind: "workbench",
        title: "Counter",
        pluginDisplayName: "Panel",
        logo: null,
      },
      {
        pluginId: "official/acme.portal",
        kind: "webview",
        title: "Developer",
        pluginDisplayName: "Portal",
        logo: null,
      },
      {
        pluginId: "official/ora-space.tools",
        kind: "webview",
        title: "X",
        pluginDisplayName: "Tools",
        logo: null,
      },
    ]);
  });

  it("returns an empty list when no surface plugin is installed", () => {
    expect(listSurfaceDefinitions([agentPlugin])).toEqual([]);
  });
});
