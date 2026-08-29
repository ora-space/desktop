import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createChatStore } from "@ora/chat";
import type { InstalledPlugin } from "@ora/contracts";
import { beforeEach, describe, expect, it } from "vitest";
import { PlatformProvider } from "../../platform";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { createSurfaceTestPlatform } from "../../test/surface-test-platform";
import { useSurfaceStore } from "../../state/stores/surface-store";
import { TRACE_DASHBOARD_PLUGIN_ID } from "../trace-dashboard/constants";
import { SurfaceLauncher } from "./surface-launcher";

function traceDashboardPlugin(): InstalledPlugin {
  return {
    id: TRACE_DASHBOARD_PLUGIN_ID,
    namespace: "official",
    name: "ora-space.agent-trace-visualizer",
    displayName: "Agent Trace Visualizer",
    description: "Agent trace visualization dashboard",
    homepage: null,
    license: null,
    version: "0.1.0",
    kind: "workbench",
    title: "Agent Trace Visualizer",
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "stopped",
  };
}

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
    startUrl: `https://example.test/${id}`,
    logo: null,
    installationValidity: { validity: "valid" },
    configuration: { state: "not_declared" },
    runtime: "stopped",
  };
}

function renderLauncher(
  plugins: InstalledPlugin[],
  embedded: boolean,
  sessionId?: string,
) {
  const state = createMockClientState();
  state.installedPlugins = plugins;
  const client = createMockClient(state);
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  const host = createSurfaceTestPlatform({ embedded });
  useSurfaceStore.getState().setEmbeddedSupported(embedded);
  render(
    <Wrapper>
      <PlatformProvider adapter={host.platform}>
        <SurfaceLauncher sessionId={sessionId ?? null} />
      </PlatformProvider>
    </Wrapper>,
  );
  return host;
}

beforeEach(() => {
  useSurfaceStore.setState({
    embeddedSupported: null,
    records: {},
    failures: {},
    sidePanelInstance: null,
  });
});

describe("SurfaceLauncher", () => {
  it("renders nothing without surface definitions", async () => {
    renderLauncher([], true);
    await waitFor(() => expect(screen.queryByRole("button")).toBeNull());
  });

  it("opens the only surface embedded and claims the side slot", async () => {
    const user = userEvent.setup();
    const host = renderLauncher(
      [webviewPlugin("ora.hub", "Hub", "Example Hub")],
      true,
    );

    await user.click(
      await screen.findByRole("button", { name: "Example Hub" }),
    );

    expect(host.surfaces.open).toHaveBeenCalledWith(
      { pluginId: "official/ora.hub" },
      "embedded",
      undefined,
    );
    await waitFor(() =>
      expect(useSurfaceStore.getState().sidePanelInstance).toBe(1),
    );
  });

  it("lists several surfaces in a menu and falls back to windowed targets", async () => {
    const user = userEvent.setup();
    const host = renderLauncher(
      [
        webviewPlugin("ora.hub", "Hub", "Example Hub"),
        webviewPlugin("ora.docs", "Docs", "Docs Home"),
      ],
      false,
    );

    await user.click(
      await screen.findByRole("button", { name: /扩展面板|Surfaces/ }),
    );
    await user.click(
      await screen.findByRole("menuitem", { name: /Example Hub/ }),
    );

    expect(host.surfaces.open).toHaveBeenCalledWith(
      { pluginId: "official/ora.hub" },
      "windowed",
      undefined,
    );
    await waitFor(() => expect(host.surfaces.open).toHaveResolved());
    expect(useSurfaceStore.getState().sidePanelInstance).toBeNull();
  });

  it("binds the trace dashboard surface to the current session when opened from a chat", async () => {
    const user = userEvent.setup();
    const host = renderLauncher(
      [traceDashboardPlugin(), webviewPlugin("ora.docs", "Docs", "Docs Home")],
      true,
      "sess-9",
    );

    await user.click(
      await screen.findByRole("button", { name: /扩展面板|Surfaces/ }),
    );
    await user.click(
      await screen.findByRole("menuitem", { name: /Agent Trace Visualizer/ }),
    );
    expect(host.surfaces.open).toHaveBeenCalledWith(
      { pluginId: TRACE_DASHBOARD_PLUGIN_ID },
      "embedded",
      "sess-9",
    );

    // Non-dashboard surfaces stay unbound even when a session is selected.
    await user.click(
      await screen.findByRole("button", { name: /扩展面板|Surfaces/ }),
    );
    await user.click(
      await screen.findByRole("menuitem", { name: /Docs Home/ }),
    );
    expect(host.surfaces.open).toHaveBeenLastCalledWith(
      { pluginId: "official/ora.docs" },
      "embedded",
      undefined,
    );
  });
});
