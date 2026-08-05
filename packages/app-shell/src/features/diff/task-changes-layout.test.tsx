import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PlatformProvider } from "@ora/platform";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { createStubPlatform } from "../../test/stub-platform";
import { useTaskChangesNavigation } from "./task-changes-navigation-context";
import { WorkspaceReviewLayout } from "../workspace/workspace-review-layout";

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
  projectRootPath: "C:/project",
};

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
      screen.getByRole("group", { name: /工作区审查面板|Workspace review panels/ }).parentElement,
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
      screen.getByRole("group", { name: /工作区审查面板|Workspace review panels/ }),
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

    expect(screen.getByRole("region", { name: "Task diff" })).toBeInTheDocument();
    expect(screen.getByTestId("requested-file")).toHaveTextContent("src/main.ts");
  });

  it("shows only Files for a project review context", async () => {
    const user = userEvent.setup();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout context={{ kind: "project", projectId: "project-1", projectRootPath: "C:/project" }}>
            <main>Project</main>
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    expect(screen.queryByRole("button", { name: /变更|Changes/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /文件|Files/ })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /文件|Files/ }));
    expect(screen.getByRole("region", { name: "Files panel" })).toBeInTheDocument();
  });

  it("keeps the files panel open and remounts it for a compatible target change", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout context={{ kind: "project", projectId: "project-1", projectRootPath: "C:/project" }}>
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
    expect(screen.getByRole("region", { name: "Task diff" })).toBeInTheDocument();

    rerender(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout context={{ kind: "project", projectId: "project-1", projectRootPath: "C:/project" }}>
            <main>Project</main>
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );
    expect(screen.queryByRole("region", { name: "Task diff" })).not.toBeInTheDocument();
    expect(screen.getByText("Project")).toBeInTheDocument();
  });
});
