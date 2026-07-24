import { render, screen, waitFor } from "@testing-library/react";
import { TooltipProvider } from "@ora/ui";
import { AppI18nProvider } from "../../i18n/i18n";
import { createStubPlatform } from "../../test/stub-platform";
import { PlatformProvider } from "@ora/platform";
import { describe, expect, it, vi } from "vitest";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { TraceDashboardPanel } from "./trace-dashboard-panel";
import type { DashboardResolver } from "./types";

function PanelShell({ resolve }: { resolve: DashboardResolver }) {
  return (
    <PlatformProvider adapter={createStubPlatform()}>
      <AppI18nProvider>
        <TooltipProvider>
          <TraceDashboardPanel resolveDashboardUrl={resolve} />
        </TooltipProvider>
      </AppI18nProvider>
    </PlatformProvider>
  );
}

function renderPanel(resolve: DashboardResolver) {
  return render(<PanelShell resolve={resolve} />);
}

describe("TraceDashboardPanel", () => {
  it("renders the panel when open with no session selected", () => {
    useUiStore.getState().setDashboardOpen(true);
    useWorkspaceSelectionStore.getState().clearSelection();
    renderPanel(vi.fn());
    // The panel header (dashboard.title) renders in both zh-CN and en-US.
    // zh-CN: "侧边面板", en-US: "Side panel" — match the common word "panel"/"面板".
    expect(screen.getByText(/panel|面板/i)).toBeInTheDocument();
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

    const iframe = await screen.findByTitle("Dashboard", undefined, { timeout: 2000 });
    expect(iframe).toHaveAttribute(
      "src",
      "http://127.0.0.1:8601/?session_id=sess-1&agent_type=claude_code",
    );
    expect(resolve).toHaveBeenCalledWith("sess-1");
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
});
