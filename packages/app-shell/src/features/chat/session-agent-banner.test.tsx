import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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

/** Builds one installed agent package whose eligibility the test controls. */
function agentPlugin(
  enabled: boolean,
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
    enabled,
    logo: null,
    ...runtime,
  };
}

/** Builds one running session bound to the given agent identity. */
function session(agentRef: string): Session {
  return {
    id: "session-1",
    taskId: "task-1",
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
  // The mock flips eligibility on the very objects it later hands back, which no
  // IPC boundary would do: react-query would see one unchanged reference and
  // skip the render that clears the banner. Copying restores the real contract
  // that every response is fresh data.
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
  return state;
}

describe("SessionAgentBanner", () => {
  it("warns and offers repair when the session's plugin is disabled", async () => {
    const state = renderBanner(
      [agentPlugin(false, { runtime: "stopped" })],
      session("ora-space.reviewer"),
    );

    const alert = await screen.findByRole("alert");
    expect(alert).toHaveAttribute("data-agent-availability", "disabled");
    expect(alert).toHaveTextContent("Code Reviewer");

    await userEvent.click(within(alert).getByRole("button"));

    expect(state.installedPlugins[0]?.enabled).toBe(true);
    // The repaired plugin makes the agent servable again, so the warning clears
    // itself once the invalidated query answers rather than waiting for the
    // next navigation.
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });

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

  it("stays silent while an enabled plugin is serving the session", async () => {
    renderBanner([agentPlugin(true)], session("ora-space.reviewer"));

    await screen.findByTestId("availability-settled");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("surfaces the backend's reason when the plugin failed to start", async () => {
    renderBanner(
      [
        agentPlugin(true, {
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
