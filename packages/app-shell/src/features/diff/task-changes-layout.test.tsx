import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PlatformProvider } from "../../platform";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { createStubPlatform } from "../../test/stub-platform";
import { useTaskChangesNavigation } from "./task-changes-navigation-context";
import { WorkspaceReviewLayout } from "../workspace/workspace-review-layout";
import { responsiveReviewWidth } from "../workspace/workspace-review-layout-utils";

vi.mock("./task-diff-view", () => ({
  TaskDiffView: ({
    toolbar,
    fileRequest,
  }: {
    toolbar?: ReactNode;
    fileRequest?: { path: string; requestId: number };
  }) => (
    <section aria-label="Task diff">
      <header data-diff-toolbar>
        <button type="button">Commit</button>
        {toolbar}
      </header>
      <span data-testid="requested-file">{fileRequest?.path}</span>
    </section>
  ),
}));

vi.mock("../files/workspace-review-files-panel", () => ({
  WorkspaceReviewFilesPanel: ({
    toolbar,
    taskId,
    projectId,
  }: {
    toolbar?: ReactNode;
    taskId?: string;
    projectId: string;
  }) => (
    <section aria-label="Files panel" data-testid="files-panel">
      <span data-testid="files-target">{taskId ?? projectId}</span>
      {toolbar}
    </section>
  ),
}));

/** Requests a file through the same context used by answer-level diff summaries. */
function OpenChangedFileButton() {
  const navigation = useTaskChangesNavigation();
  return (
    <button type="button" onClick={() => navigation?.openFile("src/main.ts")}>
      Open changed file
    </button>
  );
}

const taskContext = {
  kind: "task" as const,
  taskId: "task-1",
  projectId: "project-1",
};

describe("responsiveReviewWidth", () => {
  it("narrows the opening panel on small windows", () => {
    expect(responsiveReviewWidth(960)).toBe(460);
    expect(responsiveReviewWidth(1120)).toBe(460);
  });

  it("keeps the fuller ratio once the host is maximized-wide", () => {
    expect(responsiveReviewWidth(1600)).toBe(720);
    expect(responsiveReviewWidth(2600)).toBe(1170);
  });

  it("keeps the conversation floor on very narrow windows", () => {
    expect(responsiveReviewWidth(800)).toBe(440);
  });

  it("falls back to the default when the host cannot be measured", () => {
    expect(responsiveReviewWidth(0)).toBe(640);
  });
});

describe("WorkspaceReviewLayout", () => {
  it("positions the closed Changes trigger at the diff-toolbar coordinates", () => {
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout context={taskContext}>
            <main>Workspace</main>
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    expect(
      screen.getByRole("group", {
        name: /工作区审查面板|Workspace review panels/,
      }).parentElement,
    ).toHaveClass("right-4", "top-2");
  });

  it("moves the Changes controls beside Commit after opening", async () => {
    const user = userEvent.setup();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout context={taskContext}>
            <main>Workspace</main>
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    await user.click(screen.getByRole("button", { name: /变更|Changes/ }));

    const toolbar = document.querySelector("[data-diff-toolbar]");
    expect(toolbar).toContainElement(
      screen.getByRole("button", { name: "Commit" }),
    );
    expect(toolbar).toContainElement(
      screen.getByRole("group", {
        name: /工作区审查面板|Workspace review panels/,
      }),
    );
    expect(
      screen.getByRole("button", {
        name: /显示或隐藏变更文件目录|toggle file tree/i,
      }),
    ).toBeInTheDocument();
  });

  it("opens the Changes panel and forwards a requested answer file", async () => {
    const user = userEvent.setup();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout context={taskContext}>
            <OpenChangedFileButton />
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    await user.click(screen.getByRole("button", { name: "Open changed file" }));

    expect(
      screen.getByRole("region", { name: "Task diff" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("requested-file")).toHaveTextContent(
      "src/main.ts",
    );
  });

  it("shows only Files for a project review context", async () => {
    const user = userEvent.setup();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout
            context={{ kind: "project", projectId: "project-1" }}
          >
            <main>Project</main>
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    expect(
      screen.queryByRole("button", { name: /变更|Changes/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /文件|Files/ }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /文件|Files/ }));
    expect(
      screen.getByRole("region", { name: "Files panel" }),
    ).toBeInTheDocument();
  });

  it("keeps the files panel open and remounts it for a compatible target change", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout
            context={{ kind: "project", projectId: "project-1" }}
          >
            <main>Project</main>
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );
    await user.click(screen.getByRole("button", { name: /文件|Files/ }));
    expect(screen.getByTestId("files-target")).toHaveTextContent("project-1");

    rerender(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout context={taskContext}>
            <main>Task</main>
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );
    expect(screen.getByTestId("files-target")).toHaveTextContent("task-1");
  });

  it("closes a task-only Changes panel when switching to a project", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout context={taskContext}>
            <main>Task</main>
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );
    await user.click(screen.getByRole("button", { name: /变更|Changes/ }));
    expect(
      screen.getByRole("region", { name: "Task diff" }),
    ).toBeInTheDocument();

    rerender(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout
            context={{ kind: "project", projectId: "project-1" }}
          >
            <main>Project</main>
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );
    expect(
      screen.queryByRole("region", { name: "Task diff" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("Project")).toBeInTheDocument();
  });

  it("notifies when the review panel opens or closes", async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout
            context={taskContext}
            onOpenChange={onOpenChange}
          >
            <main>Workspace</main>
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    await user.click(
      within(
        screen.getByRole("group", {
          name: /工作区审查面板|Workspace review panels/,
        }),
      ).getByRole("button", { name: /^变更$|^Changes$/ }),
    );
    expect(onOpenChange).toHaveBeenCalledWith(true);

    await user.click(
      within(
        screen.getByRole("group", {
          name: /工作区审查面板|Workspace review panels/,
        }),
      ).getByRole("button", { name: /^变更$|^Changes$/ }),
    );
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });
});
