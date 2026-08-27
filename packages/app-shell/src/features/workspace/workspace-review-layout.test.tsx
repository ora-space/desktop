import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { PlatformProvider } from "../../platform";
import { useSurfaceStore } from "../../state/stores/surface-store";
import { createSurfaceTestPlatform } from "../../test/surface-test-platform";
import { WorkspaceReviewLayout } from "./workspace-review-layout";

vi.mock("../diff/task-diff-view", () => ({
  TaskDiffView: ({ toolbar }: { toolbar?: ReactNode }) => (
    <section aria-label="Task diff">{toolbar}</section>
  ),
}));

vi.mock("../files/workspace-review-files-panel", () => ({
  WorkspaceReviewFilesPanel: ({ toolbar }: { toolbar?: ReactNode }) => (
    <section aria-label="Files panel">{toolbar}</section>
  ),
}));

const taskContext = {
  kind: "task" as const,
  taskId: "task-1",
  projectId: "project-1",
  workspaceId: "workspace-1",
};

function renderLayout() {
  const host = createSurfaceTestPlatform({ embedded: true });
  render(
    <AppI18nProvider>
      <PlatformProvider adapter={host.platform}>
        <WorkspaceReviewLayout context={taskContext}>
          <div>conversation</div>
        </WorkspaceReviewLayout>
      </PlatformProvider>
    </AppI18nProvider>,
  );
  return host;
}

/** Seeds the store the way `useOpenSurface` does after a successful embedded open. */
function showSurface(instance: number) {
  act(() => {
    useSurfaceStore.getState().applyEvent({
      type: "opened",
      instance,
      pluginId: "ora.hub",
      kind: "webview" as const,
      target: "embedded",
      title: "Example Hub",
    });
    useSurfaceStore.getState().setSidePanelInstance(instance);
  });
}

beforeEach(() => {
  useSurfaceStore.setState({
    embeddedSupported: true,
    records: {},
    failures: {},
    sidePanelInstance: null,
  });
});

describe("WorkspaceReviewLayout surface panel", () => {
  it("shows the surface host when the store claims the slot and closes it from the header", async () => {
    const user = userEvent.setup();
    const host = renderLayout();
    expect(screen.queryByTestId("surface-placeholder")).toBeNull();

    showSurface(3);

    expect(screen.getByTestId("surface-placeholder")).toBeInTheDocument();
    expect(screen.getByText("Example Hub")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /关闭|Close/ }));
    expect(host.surfaces.close).toHaveBeenCalledWith(3);
  });

  it("collapses the panel when the host reports the surface closed", () => {
    renderLayout();
    showSurface(3);

    act(() => {
      useSurfaceStore.getState().applyEvent({ type: "closed", instance: 3 });
    });

    expect(screen.queryByTestId("surface-placeholder")).toBeNull();
    expect(useSurfaceStore.getState().sidePanelInstance).toBeNull();
  });

  it("releases the surface when the user switches to another review panel", async () => {
    const user = userEvent.setup();
    const host = renderLayout();
    showSurface(3);

    await user.click(screen.getByRole("button", { name: /^(变更|Changes)$/ }));

    expect(host.surfaces.close).toHaveBeenCalledWith(3);
    expect(useSurfaceStore.getState().sidePanelInstance).toBeNull();
    expect(screen.queryByTestId("surface-placeholder")).toBeNull();
    expect(screen.getByLabelText("Task diff")).toBeInTheDocument();
  });
});
