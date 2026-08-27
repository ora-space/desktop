import { render, screen } from "@testing-library/react";
import type { ContractsClient, InstalledPlugin, Session } from "@ora/contracts";
import { createChatStore } from "@ora/chat";
import { describe, expect, it } from "vitest";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { useAgentRuntimeStatus } from "../../state/hooks/use-agent-runtime-status";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";
import { SessionAgentBanner } from "./session-agent-banner";

/**
 * The runtime half of an installed package.
 *
 * Spelled out rather than derived: the contract models it as a tagged union on
 * the whole plugin, so `Pick` cannot isolate the arm that carries a reason.
 */
type PluginRuntime =
  | { runtime: "stopped" }
  | { runtime: "starting" }
  | { runtime: "running" }
  | { runtime: "failed"; failureReason: string };

/** Builds one installed agent package with the requested runtime state. */
function agentPlugin(
  runtime: PluginRuntime = { runtime: "running" },
): InstalledPlugin {
  return {
    id: "official/ora-space.reviewer",
    namespace: "official",
    name: "ora-space.reviewer",
    description: "ora-space.reviewer plugin",
    homepage: null,
    license: null,
    displayName: "Code Reviewer",
    version: "0.1.0",
    kind: "agent",
    agentDisplayName: "Review Agent",
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    ...runtime,
  };
}

/** Builds one running session bound to the given agent identity. */
function session(agentRef: string): Session {
  return {
    id: "session-1",
    workspaceId: "workspace-task-1",
    title: "Review",
    agentRef,
    status: "running",
    historyState: { type: "writable" },
  };
}

/**
 * Marks the point at which both availability queries have answered.
 *
 * A banner that renders nothing is only meaningful once the data it reads has
 * arrived, so the negative assertions wait for this rather than for a timeout.
 */
function SettleProbe() {
  const plugins = useInstalledPlugins();
  const supervised = useAgentRuntimeStatus();
  if (!plugins.isSuccess || !supervised.isSuccess) return null;
  return <span data-testid="availability-settled" />;
}

/** Renders the banner over a mock backend seeded with the given packages. */
function renderBanner(plugins: InstalledPlugin[], bound: Session) {
  const state = createMockClientState();
  state.installedPlugins = plugins;
  const backend = createMockClient(state);
  const client: ContractsClient = {
    ...backend,
    plugin: {
      ...backend.plugin,
      listInstalled: async (request) => ({
        plugins: (await backend.plugin.listInstalled(request)).plugins.map(
          (plugin) => ({ ...plugin }),
        ),
      }),
    },
  };
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  render(
    <Wrapper>
      <SessionAgentBanner session={bound} />
      <SettleProbe />
    </Wrapper>,
  );
}

describe("SessionAgentBanner", () => {
  it("reports an agent whose package is gone as uninstalled", async () => {
    renderBanner([], session("ora-space.reviewer"));

    expect(await screen.findByRole("alert")).toHaveAttribute(
      "data-agent-availability",
      "uninstalled",
    );
  });

  it("stays silent for a built-in CLI, which has no plugin package", async () => {
    renderBanner([], session("ora-space.nga"));

    await screen.findByTestId("availability-settled");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("stays silent while an installed plugin is serving the session", async () => {
    renderBanner([agentPlugin()], session("ora-space.reviewer"));

    await screen.findByTestId("availability-settled");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("surfaces the backend's reason when the plugin failed to start", async () => {
    renderBanner(
      [
        agentPlugin({
          runtime: "failed",
          failureReason: "deno exited with 1",
        }),
      ],
      session("ora-space.reviewer"),
    );

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveAttribute("data-agent-availability", "failed");
    // The backend's reason is the only description of what actually broke.
    expect(alert).toHaveTextContent("deno exited with 1");
  });
});
