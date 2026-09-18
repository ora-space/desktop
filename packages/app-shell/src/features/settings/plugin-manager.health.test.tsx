import { render, screen } from "@testing-library/react";
import type {
  ContractsClient,
  InstalledPlugin,
  McpHealthEntry,
} from "@ora/contracts";
import { createChatStore } from "@ora/chat";
import { describe, expect, it } from "vitest";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createPluginMemory,
  pluginHandlers,
  seedMcpHealth,
} from "../../test/memory/plugins";
import "../../i18n/i18n-instance";
import { PluginManager } from "./plugin-manager";

/** Builds one installed MCP package with the requested configuration completeness. */
function mcpPlugin(
  id: string,
  configuration: InstalledPlugin["configuration"],
): InstalledPlugin {
  const name = id.split("/")[1] ?? id;
  return {
    id,
    namespace: "official",
    name,
    displayName: name,
    version: "1.0.0",
    description: `${name} MCP server`,
    homepage: null,
    license: null,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration,
    runtime: "stopped",
    kind: "mcp",
  };
}

/** Builds one card-view health result for the named plugin. */
function cardEntry(pluginId: string): McpHealthEntry {
  return {
    identity: {
      pluginId,
      packageVersion: "1.0.0",
      configurationRevision: 1,
      transport: "stdio",
      cwd: null,
    },
    status: { status: "unhealthy", error_code: "mcp_spawn_failed" },
  };
}

/** Renders the installed-plugin manager over a mock backend seeded with the given packages. */
function renderManager(plugins: InstalledPlugin[], entries: McpHealthEntry[]) {
  const state = createPluginMemory();
  state.installedPlugins = plugins;
  seedMcpHealth(state, null, entries);
  const client: ContractsClient = createTestClient(
    pluginHandlers(state) as TestHandlers,
  );
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  render(
    <Wrapper>
      <PluginManager
        plugins={plugins}
        onBack={() => undefined}
        onConfigure={() => undefined}
        onImport={() => undefined}
        importing={false}
      />
    </Wrapper>,
  );
}

describe("PluginManager MCP health row", () => {
  it("adds Host health as a third row only for a configuration-complete MCP", async () => {
    const complete = mcpPlugin("official/ready", {
      state: "available",
      completeness: "complete",
    });
    renderManager([complete], [cardEntry(complete.id)]);

    // Install state, configuration completeness, and Host health stay three separate facts.
    const slot = await screen.findByText(/无法启动|mcp_spawn_failed/);
    expect(slot).toBeDefined();
    expect(document.querySelector("[data-slot='mcp-health']")).toHaveAttribute(
      "data-mcp-health",
      "unhealthy",
    );
  });

  it("shows no health row while configuration is still incomplete", async () => {
    const incomplete = mcpPlugin("official/pending", {
      state: "available",
      completeness: "incomplete",
    });
    renderManager([incomplete], [cardEntry(incomplete.id)]);

    // The card keeps exactly the existing "needs configuration" presentation.
    await screen.findByText(/需要配置|Needs Configuration/);
    expect(document.querySelector("[data-slot='mcp-health']")).toBeNull();
  });

  it("adds Host health for an MCP that declares no Settings", async () => {
    // Declaring no Settings is eligibility, not incompleteness: the Host probes this member, so
    // its card must show the third row instead of hiding it behind the configuration gate.
    const simple = mcpPlugin("official/simple", { state: "not_declared" });
    renderManager([simple], [cardEntry(simple.id)]);

    await screen.findByText(/无法启动|mcp_spawn_failed/);
    expect(document.querySelector("[data-slot='mcp-health']")).toHaveAttribute(
      "data-mcp-health",
      "unhealthy",
    );
  });

  it("shows no health row while configuration is unavailable", async () => {
    const unavailable = mcpPlugin("official/broken", {
      state: "unavailable",
      errorCode: "configuration_load_failed",
    });
    renderManager([unavailable], [cardEntry(unavailable.id)]);

    await screen.findByText(/配置不可用|Configuration unavailable/);
    expect(document.querySelector("[data-slot='mcp-health']")).toBeNull();
  });
});
