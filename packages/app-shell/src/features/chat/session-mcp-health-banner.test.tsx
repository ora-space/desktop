import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type {
  ContractsClient,
  InstalledPlugin,
  McpHealthEntry,
  Session,
  SessionMcpSelection,
} from "@ora/contracts";
import { createChatStore } from "@ora/chat";
import { describe, expect, it, vi } from "vitest";
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
import { PlatformProvider } from "../../platform";
import { createStubPlatform } from "../../test/stub-platform";
import "../../i18n/i18n-instance";
import { useMcpHealth } from "../../state/hooks/use-mcp-health";
import { useUiStore } from "../../state/stores/ui-store";
import { SessionMcpHealthBanner } from "./session-mcp-health-banner";
import { Composer } from "./composer";

/** The workspace directory this fixture's platform reports for every workspace. */
const WORKSPACE_CWD = "/ws/project";

/** Builds one installed MCP package whose configuration is complete. */
function mcpPlugin(id: string, displayName: string): InstalledPlugin {
  const name = id.split("/")[1] ?? id;
  return {
    id,
    namespace: "official",
    name,
    displayName,
    version: "1.0.0",
    description: `${name} MCP server`,
    homepage: null,
    license: null,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "available", completeness: "complete" },
    runtime: "stopped",
    kind: "mcp",
  };
}

/** Builds one Host health entry for the Session view. */
function entry(
  pluginId: string,
  status: McpHealthEntry["status"],
): McpHealthEntry {
  return {
    identity: {
      pluginId,
      packageVersion: "1.0.0",
      configurationRevision: 1,
      transport: "stdio",
      cwd: WORKSPACE_CWD,
    },
    status,
  };
}

/** Builds one persisted session with the requested MCP authorization. */
function session(mcpSelection: SessionMcpSelection): Session {
  return {
    id: "session-1",
    workspaceId: "workspace-1",
    title: "Review",
    agentRef: "official/opencode",
    status: "running",
    historyState: { type: "writable" },
    mcpSelection,
  };
}

/**
 * Marks the point at which the health query answered, so "no banner" is only asserted once the
 * data the banner reads has actually arrived.
 */
function HealthSettled() {
  const health = useMcpHealth(WORKSPACE_CWD);
  if (!health.isSuccess) return null;
  return <span data-testid="mcp-health-settled" />;
}

/** Renders the banner over a mock backend seeded with the given packages and health entries. */
function renderBanner(options: {
  plugins?: InstalledPlugin[];
  entries?: McpHealthEntry[];
  bound: Session;
}) {
  const state = createPluginMemory();
  state.installedPlugins = options.plugins ?? [];
  seedMcpHealth(state, WORKSPACE_CWD, options.entries ?? []);
  const backendHandlers: TestHandlers = pluginHandlers(state);
  const client: ContractsClient = createTestClient(backendHandlers);
  const stub = createStubPlatform();
  const platform = {
    ...stub,
    locationActions: {
      ...stub.locationActions,
      resolveWorkspaceCwd: async () => WORKSPACE_CWD,
    },
  };
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  render(
    <Wrapper>
      <PlatformProvider adapter={platform}>
        <SessionMcpHealthBanner session={options.bound} />
        <HealthSettled />
      </PlatformProvider>
    </Wrapper>,
  );
}

describe("SessionMcpHealthBanner", () => {
  const unhealthy = entry("official/unreachable", {
    status: "unhealthy",
    error_code: "mcp_http_unauthorized",
  });

  it("reports an unreachable server in an automatically discovered session", async () => {
    renderBanner({
      plugins: [mcpPlugin("official/unreachable", "Unreachable MCP")],
      entries: [unhealthy],
      bound: session({ mode: "automatic" }),
    });

    const banner = await screen.findByRole("status");
    expect(banner).toHaveAttribute("data-mcp-health-banner", "unhealthy");
    // The member and its stable code name what the Host could not reach.
    expect(banner).toHaveTextContent("Unreachable MCP");
    expect(banner).toHaveTextContent("mcp_http_unauthorized");
    // Health is Host-observed: the text never promises in-session effect.
    expect(banner).not.toHaveTextContent(/MCP Ready/);
    expect(banner).not.toHaveTextContent(/已在会话内生效/);
  });

  it("hides an unhealthy server the session never selected", async () => {
    renderBanner({
      plugins: [mcpPlugin("official/unreachable", "Unreachable MCP")],
      entries: [unhealthy],
      bound: session({ mode: "explicit", pluginIds: ["official/other"] }),
    });

    await screen.findByTestId("mcp-health-settled");
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("shows nothing for an explicit empty selection", async () => {
    renderBanner({
      plugins: [mcpPlugin("official/unreachable", "Unreachable MCP")],
      entries: [unhealthy],
      bound: session({ mode: "explicit", pluginIds: [] }),
    });

    await screen.findByTestId("mcp-health-settled");
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("does not treat a pending probe as a failure", async () => {
    renderBanner({
      plugins: [mcpPlugin("official/pending", "Pending MCP")],
      entries: [
        entry("official/pending", {
          status: "unknown",
          reason: "not_probed",
        }),
      ],
      bound: session({ mode: "automatic" }),
    });

    await screen.findByTestId("mcp-health-settled");
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("sends the user to the configuration entry for the unreachable server", async () => {
    const user = userEvent.setup();
    const previous = useUiStore.getState();
    renderBanner({
      plugins: [mcpPlugin("official/unreachable", "Unreachable MCP")],
      entries: [unhealthy],
      bound: session({ mode: "automatic" }),
    });

    await screen.findByRole("status");
    await user.click(screen.getByRole("button", { name: /配置|Configure/ }));
    // The action is a deep link into the existing plugin configuration pane, not a second editor.
    expect(useUiStore.getState().pluginSettingsRequest).toEqual({
      kind: "configure",
      pluginId: "official/unreachable",
      displayName: "Unreachable MCP",
    });
    expect(useUiStore.getState().settingsOpen).toBe(true);

    useUiStore.setState({
      settingsOpen: previous.settingsOpen,
      pluginSettingsRequest: previous.pluginSettingsRequest,
    });
  });

  it("keeps the prompt editable and sendable while the banner is shown", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    const state = createPluginMemory();
    state.installedPlugins = [
      mcpPlugin("official/unreachable", "Unreachable MCP"),
    ];
    seedMcpHealth(state, WORKSPACE_CWD, [
      entry("official/unreachable", {
        status: "unhealthy",
        error_code: "mcp_http_unauthorized",
      }),
    ]);
    const backendHandlers: TestHandlers = pluginHandlers(state);
    const client: ContractsClient = createTestClient(backendHandlers);
    const stub = createStubPlatform();
    const platform = {
      ...stub,
      locationActions: {
        ...stub.locationActions,
        resolveWorkspaceCwd: async () => WORKSPACE_CWD,
      },
    };
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
    );
    render(
      <Wrapper>
        <PlatformProvider adapter={platform}>
          <SessionMcpHealthBanner session={session({ mode: "automatic" })} />
          <Composer onSend={onSend} isResponding={false} />
        </PlatformProvider>
      </Wrapper>,
    );

    // The banner is a non-blocking status surface: the composer keeps accepting prompts.
    await screen.findByRole("status");
    const textarea = screen.getByRole("textbox");
    expect(textarea).toBeEnabled();
    await user.type(textarea, "hello{Enter}");
    expect(onSend).toHaveBeenCalledWith("hello");
  });
});
