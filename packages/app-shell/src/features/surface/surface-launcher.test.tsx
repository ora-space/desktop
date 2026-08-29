import { act, render, screen, waitFor } from "@testing-library/react";
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
      [webviewPlugin("ora.hub", "Hub", "Example Hub")],
      true,
    );

    await user.click(
      await screen.findByRole("button", { name: "Example Hub" }),
    );

    expect(host.surfaces.open).toHaveBeenCalledWith(
      { pluginId: "official/ora.hub" },
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
    );
    await waitFor(() => expect(host.surfaces.open).toHaveResolved());
    expect(useSurfaceStore.getState().sidePanelInstance).toBeNull();
  });

  it("marks live embedded instances with the slot owner highlighted", async () => {
    const user = userEvent.setup();
    renderLauncher(
      [
        {
          ...webviewPlugin("ora.hub", "Hub", "Example Hub"),
          logo: "<svg xmlns='http://www.w3.org/2000/svg'></svg>",
        },
        webviewPlugin("ora.docs", "Docs", "Docs Home"),
      ],
      true,
    );
    await user.click(
      await screen.findByRole("button", { name: /扩展面板|Surfaces/ }),
    );

    // Two live embedded instances; Docs owns the side slot right now.
    act(() => {
      useSurfaceStore.setState({
        records: {
          1: {
            instance: 1,
            pluginId: "official/ora.hub",
            kind: "webview" as const,
            title: "Example Hub",
            target: "embedded" as const,
            state: "open" as const,
          },
          2: {
            instance: 2,
            pluginId: "official/ora.docs",
            kind: "webview" as const,
            title: "Docs Home",
            target: "embedded" as const,
            state: "open" as const,
          },
        },
        sidePanelInstance: 2,
      });
    });

    const hubItem = await screen.findByRole("menuitem", {
      name: /Example Hub/,
    });
    const docsItem = await screen.findByRole("menuitem", {
      name: /Docs Home/,
    });
    // The logo ships as inline SVG source, rendered through an inert <img>.
    expect(hubItem.querySelector("img")).not.toBeNull();
    const hubDot = hubItem.querySelector(".rounded-full");
    const docsDot = docsItem.querySelector(".rounded-full");
    expect(hubDot).toHaveClass("bg-muted-foreground/40");
    expect(docsDot).toHaveClass("bg-foreground");
  });
});
