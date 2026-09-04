import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { PlatformProvider } from "../../platform";
import { useSurfaceStore } from "../../state/stores/surface-store";
import { createSurfaceTestPlatform } from "../../test/surface-test-platform";
import { SessionDashboardButton } from "./session-dashboard-button";

beforeEach(() => {
  useSurfaceStore.setState({
    embeddedSupported: null,
    records: {},
    failures: {},
    sidePanelInstance: null,
  });
});

describe("SessionDashboardButton", () => {
  it("renders only for a persisted session", () => {
    const host = createSurfaceTestPlatform({ embedded: true });
    const { rerender } = render(
      <PlatformProvider adapter={host.platform}>
        <SessionDashboardButton sessionId={null} />
      </PlatformProvider>,
    );
    expect(screen.queryByRole("button")).toBeNull();

    rerender(
      <PlatformProvider adapter={host.platform}>
        <SessionDashboardButton sessionId="session-1" />
      </PlatformProvider>,
    );
    expect(screen.getByRole("button")).toBeInTheDocument();
  });

  it("opens the selected session embedded and claims the side panel", async () => {
    const user = userEvent.setup();
    const host = createSurfaceTestPlatform({ embedded: true });
    useSurfaceStore.getState().setEmbeddedSupported(true);
    render(
      <PlatformProvider adapter={host.platform}>
        <SessionDashboardButton sessionId="session-1" />
      </PlatformProvider>,
    );

    await user.click(screen.getByRole("button"));

    expect(host.surfaces.openSessionTraceDashboard).toHaveBeenCalledWith(
      "session-1",
      "embedded",
    );
    await waitFor(() => {
      expect(useSurfaceStore.getState().sidePanelInstance).toBe(1);
    });
    expect(useSurfaceStore.getState().records[1]?.pluginId).toBe(
      "official/ora-space.agent-dashboard",
    );
  });

  it("falls back to a standalone window", async () => {
    const user = userEvent.setup();
    const host = createSurfaceTestPlatform({ embedded: false });
    useSurfaceStore.getState().setEmbeddedSupported(false);
    render(
      <PlatformProvider adapter={host.platform}>
        <SessionDashboardButton sessionId="session-2" />
      </PlatformProvider>,
    );

    await user.click(screen.getByRole("button"));

    expect(host.surfaces.openSessionTraceDashboard).toHaveBeenCalledWith(
      "session-2",
      "windowed",
    );
    await waitFor(() =>
      expect(host.surfaces.openSessionTraceDashboard).toHaveResolved(),
    );
    expect(useSurfaceStore.getState().sidePanelInstance).toBeNull();
  });
});
