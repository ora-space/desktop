import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PlatformProvider } from "../../platform";
import { useState, type ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { createStubPlatform } from "../../test/stub-platform";
import { useTaskChangesNavigation } from "./task-changes-navigation-context";
import { WorkspaceReviewLayout } from "../workspace/workspace-review-layout";
import { responsiveReviewWidth } from "../workspace/workspace-review-layout-utils";
import { flushDebouncedPersistStorage } from "../../state/stores/debounced-json-storage";
import {
  REVIEW_STORAGE_KEY,
  useReviewStore,
} from "../../state/stores/review-store";

vi.mock("./task-diff-view", () => ({
  TaskDiffView: ({
    toolbar,
    fileRequest,
    onFileNotFound,
    onPreviewPathChange,
  }: {
    toolbar?: ReactNode;
    fileRequest?: { path: string; requestId: number; line?: number };
    onFileNotFound?: (path: string, line?: number) => void;
    onPreviewPathChange?: (path: string) => void;
  }) => (
    <section aria-label="Task diff">
      <header data-diff-toolbar>
        <button type="button">Commit</button>
        {toolbar}
      </header>
      <span data-testid="requested-file">{fileRequest?.path}</span>
      <span data-testid="requested-line">{fileRequest?.line ?? ""}</span>
      <button
        type="button"
        data-testid="simulate-not-found"
        onClick={() => onFileNotFound?.("src/missing.ts", 10)}
      >
        Simulate Not Found
      </button>
      <button
        type="button"
        data-testid="simulate-diff-preview"
        onClick={() => onPreviewPathChange?.("src/diff-picked.ts")}
      >
        Simulate diff tree pick
      </button>
    </section>
  ),
}));

vi.mock("../files/workspace-review-files-panel", () => ({
  WorkspaceReviewFilesPanel: ({
    toolbar,
    taskId,
    projectId,
    fileRequest,
    directoryRequest,
    onPreviewPathChange,
  }: {
    toolbar?: ReactNode;
    taskId?: string;
    projectId: string;
    fileRequest?: { path: string; requestId: number; line?: number };
    directoryRequest?: { path: string; requestId: number };
    onPreviewPathChange?: (path: string) => void;
  }) => (
    <section aria-label="Files panel" data-testid="files-panel">
      <span data-testid="files-target">{taskId ?? projectId}</span>
      <span data-testid="files-request">
        {fileRequest === undefined
          ? ""
          : `${fileRequest.path}:${fileRequest.line ?? ""}`}
      </span>
      <span data-testid="files-request-id">{fileRequest?.requestId ?? ""}</span>
      <span data-testid="directory-request">
        {directoryRequest?.path ?? ""}
      </span>
      <button
        type="button"
        data-testid="simulate-files-preview"
        onClick={() => onPreviewPathChange?.("src/tree-picked.ts")}
      >
        Simulate tree pick
      </button>
      {toolbar}
    </section>
  ),
}));

/** Requests a file through the same context used by answer-level diff summaries. */
function OpenChangedFileButton() {
  const navigation = useTaskChangesNavigation();
  return (
    <button type="button" onClick={() => navigation?.openDiff("src/main.ts")}>
      Open changed file
    </button>
  );
}

/** Requests a workspace file preview the way inline referenced links do. */
function OpenWorkspaceFileButton() {
  const navigation = useTaskChangesNavigation();
  return (
    <button
      type="button"
      onClick={() => navigation?.openWorkspaceFile("src/lib.ts", 8, 1)}
    >
      Open workspace file
    </button>
  );
}

/** Requests a workspace folder the way a directory chip navigation does. */
function OpenWorkspaceFolderButton() {
  const navigation = useTaskChangesNavigation();
  return (
    <button
      type="button"
      onClick={() => navigation?.openWorkspaceDirectory?.("src/features")}
    >
      Open workspace folder
    </button>
  );
}

/** Models workspace-owned UI state that must survive opening the review panel. */
function StatefulWorkspace() {
  const [inspectorOpen, setInspectorOpen] = useState(false);
  const navigation = useTaskChangesNavigation();
  return (
    <main>
      <button type="button" onClick={() => setInspectorOpen(true)}>
        Open inspector
      </button>
      <span>{inspectorOpen ? "Inspector open" : "Inspector closed"}</span>
      <button type="button" onClick={() => navigation?.openDiff("src/main.ts")}>
        Open changed file
      </button>
    </main>
  );
}

const taskContext = {
  kind: "task" as const,
  taskId: "task-1",
  projectId: "project-1",
  workspaceId: "workspace-1",
};

beforeEach(() => {
  flushDebouncedPersistStorage();
  window.localStorage.clear();
  useReviewStore.setState({ byContext: {} });
  flushDebouncedPersistStorage();
  window.localStorage.clear();
});

/** Clicks the Changes tab in the review toolbar. */
async function clickChangesTab(user: ReturnType<typeof userEvent.setup>) {
  await user.click(
    within(
      screen.getByRole("group", {
        name: /工作区审查面板|Workspace review panels/,
      }),
    ).getByRole("button", { name: /^变更$|^Changes$/ }),
  );
}

/** Clicks the Files tab in the review toolbar. */
async function clickFilesTab(user: ReturnType<typeof userEvent.setup>) {
  await user.click(
    within(
      screen.getByRole("group", {
        name: /工作区审查面板|Workspace review panels/,
      }),
    ).getByRole("button", { name: /^文件$|^Files$/ }),
  );
}

/**
 * Rebuilds the `context` prop as a fresh object literal on every render, the way
 * `workspace-view` / `workflow-run-workspace` derive it. The button lets a test
 * force a parent render without touching the review layout's own state.
 */
function RerenderHarness() {
  const [tick, setTick] = useState(0);
  return (
    <>
      <button
        type="button"
        data-testid="force-parent-render"
        onClick={() => setTick((current) => current + 1)}
      >
        rerender {tick}
      </button>
      <WorkspaceReviewLayout
        context={{
          kind: "task",
          taskId: "task-1",
          projectId: "project-1",
          workspaceId: "workspace-1",
        }}
      >
        <main>Workspace</main>
      </WorkspaceReviewLayout>
    </>
  );
}

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

  it("opens the Files panel and forwards a workspace file request with a line", async () => {
    const user = userEvent.setup();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout context={taskContext}>
            <OpenWorkspaceFileButton />
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    await user.click(
      screen.getByRole("button", { name: "Open workspace file" }),
    );

    expect(
      screen.getByRole("region", { name: "Files panel" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("files-request")).toHaveTextContent(
      "src/lib.ts:8",
    );
  });

  it("opens the Files panel and forwards a workspace folder request", async () => {
    const user = userEvent.setup();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout context={taskContext}>
            <OpenWorkspaceFolderButton />
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    await user.click(
      screen.getByRole("button", { name: "Open workspace folder" }),
    );

    expect(
      screen.getByRole("region", { name: "Files panel" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("directory-request")).toHaveTextContent(
      "src/features",
    );
  });

  it("preserves workspace state when opening Changes", async () => {
    const user = userEvent.setup();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout
            context={taskContext}
            preserveWorkspaceOnReviewOpen
          >
            <StatefulWorkspace />
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    await user.click(screen.getByRole("button", { name: "Open inspector" }));
    expect(screen.getByText("Inspector open")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Open changed file" }));

    expect(screen.getByText("Inspector open")).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "Task diff" }),
    ).toBeInTheDocument();
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

  it("switches an open Changes panel to Files when entering project context", async () => {
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
    expect(
      screen.getByRole("button", {
        name: /显示或隐藏变更文件目录|Show or hide changed file tree/,
      }),
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
    expect(screen.getByTestId("files-panel")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", {
        name: /显示或隐藏变更文件目录|Show or hide changed file tree/,
      }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /文件|Files/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
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

  it("falls back to the Files panel when a requested file is not found in the diff", async () => {
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

    await user.click(screen.getByTestId("simulate-not-found"));
    expect(screen.getByTestId("files-panel")).toBeInTheDocument();
    expect(screen.getByTestId("files-request")).toHaveTextContent(
      "src/missing.ts:10",
    );
  });

  it("routes openDiff to Files when the review context has no task", async () => {
    const user = userEvent.setup();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout
            context={{ kind: "project", projectId: "project-1" }}
          >
            <OpenChangedFileButton />
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    await user.click(screen.getByRole("button", { name: "Open changed file" }));
    expect(screen.getByTestId("files-panel")).toBeInTheDocument();
    expect(screen.getByTestId("files-request")).toHaveTextContent(
      "src/main.ts:",
    );
    expect(
      screen.queryByRole("region", { name: "Task diff" }),
    ).not.toBeInTheDocument();
  });

  it("drops a previous task's file request when switching tasks", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout context={taskContext}>
            <OpenWorkspaceFileButton />
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    await user.click(
      screen.getByRole("button", { name: "Open workspace file" }),
    );
    expect(screen.getByTestId("files-request")).toHaveTextContent(
      "src/lib.ts:8",
    );

    rerender(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout
            context={{
              kind: "task",
              taskId: "task-2",
              projectId: "project-1",
            }}
          >
            <OpenWorkspaceFileButton />
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    expect(screen.getByTestId("files-target")).toHaveTextContent("task-2");
    expect(screen.getByTestId("files-request").textContent).toBe("");
  });

  it("restores an open Changes panel and file after review storage hydrates", async () => {
    window.localStorage.setItem(
      REVIEW_STORAGE_KEY,
      JSON.stringify({
        state: {
          byContext: {
            "task:task-1": {
              open: true,
              panel: "changes",
              width: 720,
              files: { changes: { path: "src/main.ts", line: 4 } },
            },
          },
        },
        version: 0,
      }),
    );

    await act(async () => {
      await useReviewStore.persist.rehydrate();
    });

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
      screen.getByRole("region", { name: "Task diff" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("requested-file")).toHaveTextContent(
      "src/main.ts",
    );
    expect(screen.getByTestId("requested-line")).toHaveTextContent("4");
  });

  it("re-applies per-context open state when switching tasks", async () => {
    useReviewStore.getState().upsertContext("task:task-1", {
      open: true,
      panel: "files",
      width: 640,
      files: { files: { path: "src/a.ts" } },
    });
    useReviewStore.getState().upsertContext("task:task-2", {
      open: false,
      panel: "files",
      width: 640,
    });

    await act(async () => {
      await useReviewStore.persist.rehydrate();
    });

    const { rerender } = render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout context={taskContext}>
            <main>Task 1</main>
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    expect(
      screen.getByRole("region", { name: "Files panel" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("files-request")).toHaveTextContent("src/a.ts:");

    rerender(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout
            context={{
              kind: "task",
              taskId: "task-2",
              projectId: "project-1",
            }}
          >
            <main>Task 2</main>
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    expect(screen.queryByRole("region", { name: "Files panel" })).toBeNull();

    rerender(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <WorkspaceReviewLayout context={taskContext}>
            <main>Task 1</main>
          </WorkspaceReviewLayout>
        </AppI18nProvider>
      </PlatformProvider>,
    );

    expect(
      screen.getByRole("region", { name: "Files panel" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("files-request")).toHaveTextContent("src/a.ts:");
  });

  it("opens the Files panel for a project when persisted Changes is invalid", async () => {
    useReviewStore.getState().upsertContext("project:project-1", {
      open: true,
      panel: "changes",
      width: 640,
      files: { files: { path: "README.md", line: 2 } },
    });

    await act(async () => {
      await useReviewStore.persist.rehydrate();
    });

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

    expect(screen.getByTestId("files-panel")).toBeInTheDocument();
    expect(screen.getByTestId("files-request")).toHaveTextContent(
      "README.md:2",
    );
    expect(
      screen.queryByRole("region", { name: "Task diff" }),
    ).not.toBeInTheDocument();
  });

  it("does not replace a Files preview path when switching to Changes without a selection", async () => {
    useReviewStore.getState().upsertContext("task:task-1", {
      open: true,
      panel: "files",
      width: 640,
      files: { files: { path: "src/b.ts" } },
    });

    await act(async () => {
      await useReviewStore.persist.rehydrate();
    });

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

    expect(screen.getByTestId("files-request")).toHaveTextContent("src/b.ts:");

    await clickChangesTab(user);
    flushDebouncedPersistStorage();

    expect(useReviewStore.getState().byContext["task:task-1"]?.files).toEqual({
      files: { path: "src/b.ts" },
    });
  });

  it("reopens the last previewed file after the panel was closed on the prior session", async () => {
    useReviewStore.getState().upsertContext("task:task-1", {
      open: false,
      panel: "files",
      width: 640,
      files: { files: { path: "src/closed.ts" } },
    });

    await act(async () => {
      await useReviewStore.persist.rehydrate();
    });

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

    await clickFilesTab(user);

    expect(screen.getByTestId("files-request")).toHaveTextContent(
      "src/closed.ts:",
    );
  });

  it("persists a file chosen from the tree via onPreviewPathChange", async () => {
    useReviewStore.getState().upsertContext("task:task-1", {
      open: true,
      panel: "files",
      width: 640,
    });

    await act(async () => {
      await useReviewStore.persist.rehydrate();
    });

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

    await user.click(screen.getByTestId("simulate-files-preview"));
    await act(async () => {
      flushDebouncedPersistStorage();
    });

    expect(useReviewStore.getState().byContext["task:task-1"]).toMatchObject({
      files: { files: { path: "src/tree-picked.ts" } },
    });
  });

  it("persists a diff file chosen from the tree via onPreviewPathChange", async () => {
    useReviewStore.getState().upsertContext("task:task-1", {
      open: true,
      panel: "changes",
      width: 640,
    });

    await act(async () => {
      await useReviewStore.persist.rehydrate();
    });

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

    await user.click(screen.getByTestId("simulate-diff-preview"));
    await act(async () => {
      flushDebouncedPersistStorage();
    });

    expect(useReviewStore.getState().byContext["task:task-1"]).toMatchObject({
      panel: "changes",
      files: { changes: { path: "src/diff-picked.ts" } },
    });
  });

  it("restores once per scope even when the parent re-renders", async () => {
    useReviewStore.getState().upsertContext("task:task-1", {
      open: true,
      panel: "files",
      width: 640,
      files: { files: { path: "src/a.ts" } },
    });

    await act(async () => {
      await useReviewStore.persist.rehydrate();
    });

    const user = userEvent.setup();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <RerenderHarness />
        </AppI18nProvider>
      </PlatformProvider>,
    );

    expect(screen.getByTestId("files-request")).toHaveTextContent("src/a.ts:");
    const restoredRequestId =
      screen.getByTestId("files-request-id").textContent;

    // Each parent render hands down a brand new context object. Restore must
    // stay one-shot per scope: re-running it bumps the request id, and a fresh
    // id makes the files panel re-read and re-scroll the same file — once per
    // streaming token in production.
    await user.click(screen.getByTestId("force-parent-render"));
    await user.click(screen.getByTestId("force-parent-render"));

    expect(screen.getByTestId("files-request-id")).toHaveTextContent(
      restoredRequestId ?? "",
    );
    expect(screen.getByTestId("files-request")).toHaveTextContent("src/a.ts:");
  });

  it("never adopts a Changes path as the Files panel selection", async () => {
    useReviewStore.getState().upsertContext("task:task-1", {
      open: true,
      panel: "files",
      width: 640,
      files: { files: { path: "src/a.ts" } },
    });

    await act(async () => {
      await useReviewStore.persist.rehydrate();
    });

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

    await clickChangesTab(user);
    await user.click(screen.getByTestId("simulate-diff-preview"));
    await clickFilesTab(user);
    await act(async () => {
      flushDebouncedPersistStorage();
    });

    // The diff-only path stays under `changes`; Files keeps its own selection.
    expect(useReviewStore.getState().byContext["task:task-1"]).toMatchObject({
      panel: "files",
      files: {
        files: { path: "src/a.ts" },
        changes: { path: "src/diff-picked.ts" },
      },
    });
    expect(screen.getByTestId("files-request")).toHaveTextContent("src/a.ts:");
  });
});
