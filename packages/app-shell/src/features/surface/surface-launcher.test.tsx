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
import { SurfaceLauncher } from "./surface-launcher";

function uiPlugin(
  id: string,
  displayName: string,
  surfaces: Array<{ id: string; title: string }>,
): InstalledPlugin {
  return {
    id,
    packageName: `@ora-space/${id}`,
    displayName,
    version: "0.1.0",
    main: "dist/index.js",
    kind: "ui",
    contractVersion: 1,
    surfaces: surfaces.map((surface) => ({
      ...surface,
      entryUrl: `https://example.test/${surface.id}`,
      source: "remote_site",
    })),
    enabled: true,
    runtime: "stopped",
  };
}

function renderLauncher(plugins: InstalledPlugin[], embedded: boolean) {
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
        <SurfaceLauncher />
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
      [uiPlugin("ora.hub", "Hub", [{ id: "market", title: "Skill Hub" }])],
      true,
    );

    await user.click(await screen.findByRole("button", { name: "Skill Hub" }));

    expect(host.surfaces.open).toHaveBeenCalledWith(
      { pluginId: "ora.hub", surfaceId: "market" },
      "embedded",
    );
    await waitFor(() =>
      expect(useSurfaceStore.getState().sidePanelInstance).toBe(1),
    );
  });

  it("lists several surfaces in a menu and falls back to windowed targets", async () => {
    const user = userEvent.setup();
    const host = renderLauncher(
      [
        uiPlugin("ora.hub", "Hub", [{ id: "market", title: "Skill Hub" }]),
        uiPlugin("ora.docs", "Docs", [{ id: "home", title: "Docs Home" }]),
      ],
      false,
    );

    await user.click(
      await screen.findByRole("button", { name: /扩展面板|Surfaces/ }),
    );
    await user.click(
      await screen.findByRole("menuitem", { name: /Skill Hub/ }),
    );

    expect(host.surfaces.open).toHaveBeenCalledWith(
      { pluginId: "ora.hub", surfaceId: "market" },
      "windowed",
    );
    await waitFor(() => expect(host.surfaces.open).toHaveResolved());
    expect(useSurfaceStore.getState().sidePanelInstance).toBeNull();
  });
});
