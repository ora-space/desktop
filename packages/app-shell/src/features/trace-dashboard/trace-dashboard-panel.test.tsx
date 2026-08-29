import { render, screen, waitFor } from "@testing-library/react";
import { createChatStore } from "@ora/chat";
import type { InstalledPlugin } from "@ora/contracts";
import { TooltipProvider } from "@ora/ui";
import { AppI18nProvider } from "../../i18n/i18n";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { createSurfaceTestPlatform } from "../../test/surface-test-platform";
import { PlatformProvider, type PlatformAdapter } from "../../platform";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useUiStore } from "../../state/stores/ui-store";
import { useSurfaceStore } from "../../state/stores/surface-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { TraceDashboardPanel } from "./trace-dashboard-panel";
import type { DashboardCompareResolver, DashboardResolver } from "./types";
import { TRACE_DASHBOARD_PLUGIN_ID } from "./constants";

function traceDashboardPlugin(): InstalledPlugin {
  return {
    id: TRACE_DASHBOARD_PLUGIN_ID,
    namespace: "official",
    name: "agent-trace-visualizer",
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

function PanelShell({
  resolve,
  resolveCompare = null,
  platform,
}: {
  resolve: DashboardResolver;
  resolveCompare?: DashboardCompareResolver | null;
  platform: PlatformAdapter;
}) {
  return (
    <PlatformProvider adapter={platform}>
      <AppI18nProvider>
        <TooltipProvider>
          <TraceDashboardPanel
            resolveDashboardUrl={resolve}
            resolveDashboardCompareUrl={resolveCompare}
          />
        </TooltipProvider>
      </AppI18nProvider>
    </PlatformProvider>
  );
}

function renderPanel(
  resolve: DashboardResolver,
  resolveCompare?: DashboardCompareResolver | null,
  installedPlugins: InstalledPlugin[] = [],
  surfaceHost: ReturnType<
    typeof createSurfaceTestPlatform
  > = createSurfaceTestPlatform({ embedded: true }),
) {
  const state = createMockClientState();
  state.installedPlugins = installedPlugins;
  const client = createMockClient(state);
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  return render(
    <Wrapper>
      <PanelShell
        resolve={resolve}
        resolveCompare={resolveCompare}
        platform={surfaceHost.platform}
      />
    </Wrapper>,
  );
}

describe("TraceDashboardPanel", () => {
  beforeEach(() => {
    useUiStore.setState({
      dashboardOpen: false,
      dashboardMode: "trace",
      dashboardWidth: 800,
    });
    useWorkspaceSelectionStore.getState().clearSelection();
  });

  it("renders the panel when open with no session selected", async () => {
    useUiStore.getState().setDashboardOpen(true);
    useWorkspaceSelectionStore.getState().clearSelection();
    renderPanel(vi.fn());
    // The panel is portaled by Sheet; the title (dashboard.title) renders async.
    // zh-CN: "侧边面板", en-US: "Side panel" — match the common word "panel"/"面板".
    expect(await screen.findByText(/panel|面板/i)).toBeInTheDocument();
  });

  it("renders the iframe with the resolved URL once the server is reachable", async () => {
    useUiStore.getState().setDashboardOpen(true);
    useWorkspaceSelectionStore.getState().selectSession("sess-1", "t1", "p1");
    const resolve = vi.fn(async () => ({
      host: "127.0.0.1",
      port: 8601,
      url: "http://127.0.0.1:8601/?session_id=sess-1&agent_type=claude_code",
      serverReachable: true,
    })) as unknown as DashboardResolver;

    renderPanel(resolve);

    const iframe = await screen.findByTitle("Dashboard", undefined, {
      timeout: 2000,
    });
    expect(iframe).toHaveAttribute(
      "src",
      "http://127.0.0.1:8601/?session_id=sess-1&agent_type=claude_code",
    );
    expect(resolve).toHaveBeenCalledWith("sess-1");
  });

  it("renders the token comparison iframe without a selected session", async () => {
    useUiStore.getState().openDashboardPanel("compare");
    useWorkspaceSelectionStore.getState().clearSelection();
    const resolve = vi.fn() as unknown as DashboardResolver;
    const resolveCompare = vi.fn(async () => ({
      host: "127.0.0.1",
      port: 8601,
      url: "http://127.0.0.1:8601/?app_mode=compare",
      serverReachable: true,
    })) as unknown as DashboardCompareResolver;

    renderPanel(resolve, resolveCompare);

    const iframe = await screen.findByTitle(/Token/i, undefined, {
      timeout: 2000,
    });
    expect(iframe).toHaveAttribute(
      "src",
      "http://127.0.0.1:8601/?app_mode=compare",
    );
    expect(resolveCompare).toHaveBeenCalledOnce();
    expect(resolve).not.toHaveBeenCalled();
  });

  it("shows the server-unreachable guidance when the probe reports the server down", async () => {
    useUiStore.getState().setDashboardOpen(true);
    useWorkspaceSelectionStore.getState().selectSession("sess-2", "t1", "p1");
    const resolve = vi.fn(async () => ({
      host: "127.0.0.1",
      port: 8601,
      url: "http://127.0.0.1:8601/?session_id=sess-2&agent_type=opencode",
      serverReachable: false,
    })) as unknown as DashboardResolver;

    renderPanel(resolve);

    await waitFor(() => {
      // The server-unreachable copy mentions streamlit in both locales.
      expect(screen.getByText(/streamlit/i)).toBeInTheDocument();
    });
  });

  it("applies the persisted dashboard width instead of the sheet's narrow default", async () => {
    useUiStore.getState().setDashboardOpen(true);
    useUiStore.getState().setDashboardWidth(900);
    useWorkspaceSelectionStore.getState().clearSelection();
    renderPanel(vi.fn());
    const title = await screen.findByText(/panel|面板/i);
    // The sheet content is the closest positioned ancestor holding the width style.
    const content = title.closest('[data-slot="sheet-content"]');
    expect(content).not.toBeNull();
    // 900 is within the 420–1400 clamp, so it must be applied verbatim — not capped
    // to the sheet's sm:max-w-sm (384px) default.
    const style = content?.getAttribute("style") ?? "";
    expect(style).toContain("width: 900px");
    expect(style).toContain("max-width: none");
  });

  it("opens the plugin surface bound to the session when the trace-visualizer plugin is installed", async () => {
    useUiStore.getState().setDashboardOpen(true);
    useSurfaceStore.getState().setEmbeddedSupported(true);
    useWorkspaceSelectionStore.getState().selectSession("sess-3", "t1", "p1");
    const resolve = vi.fn(async () => ({
      host: "127.0.0.1",
      port: 8601,
      url: "http://127.0.0.1:8601/?session_id=sess-3",
      serverReachable: true,
    })) as unknown as DashboardResolver;
    const host = createSurfaceTestPlatform({ embedded: true });
    renderPanel(resolve, null, [traceDashboardPlugin()], host);

    await waitFor(() => {
      expect(host.surfaces.open).toHaveBeenCalledWith(
        { pluginId: TRACE_DASHBOARD_PLUGIN_ID },
        "embedded",
        "sess-3",
      );
    });
    // The sheet hands the slot over to the native surface and closes.
    expect(useUiStore.getState().dashboardOpen).toBe(false);
    // The legacy endpoint resolver is never consulted.
    expect(resolve).not.toHaveBeenCalled();
  });
});
