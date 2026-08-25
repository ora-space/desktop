import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PlatformProvider } from "../../platform";
import { TooltipProvider } from "@ora/ui";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { createStubPlatform } from "../../test/stub-platform";
import { WorkspaceReviewFilesPanel } from "./workspace-review-files-panel";

vi.mock("../specs/specs-view", () => ({
  SpecsContent: () => <div data-testid="specs-content">Specs content</div>,
}));

vi.mock("./workspace-files-view", () => ({
  WorkspaceFilesView: ({
    surface,
    fileRequest,
    artifactRequest,
    projectId,
    taskId,
  }: {
    surface: "explorer" | "search";
    fileRequest?: { path: string; requestId: number; line?: number };
    artifactRequest?: { path: string; requestId: number; line?: number };
    projectId: string;
    taskId?: string;
  }) => (
    <div data-testid="files-explorer">
      {surface}:{projectId}:{taskId ?? ""}:{fileRequest?.path ?? ""}:
      {fileRequest?.line ?? ""}
      {artifactRequest?.path ?? ""}
    </div>
  ),
}));

function renderPanel(props: {
  projectId?: string;
  taskId?: string;
  fileRequest?: { path: string; requestId: number; line?: number };
  artifactRequest?: { path: string; requestId: number; line?: number };
}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TooltipProvider>
            <WorkspaceReviewFilesPanel
              projectId={props.projectId ?? "project-1"}
              taskId={props.taskId}
              fileRequest={props.fileRequest}
              artifactRequest={props.artifactRequest}
            />
          </TooltipProvider>
        </AppI18nProvider>
      </PlatformProvider>
    </QueryClientProvider>,
  );
}

describe("WorkspaceReviewFilesPanel", () => {
  it("opens task files on explorer and exposes specs refresh only in the specs sub-view", async () => {
    const user = userEvent.setup();
    renderPanel({ taskId: "task-1" });

    expect(screen.getByTestId("files-explorer")).toHaveTextContent("explorer");
    await user.click(screen.getByRole("button", { name: "Specs" }));
    expect(screen.getByTestId("specs-content")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /刷新 Specs|Refresh Specs/ }),
    ).toBeInTheDocument();
  });

  it("opens project files on explorer with search available when no task is selected", () => {
    renderPanel({});

    expect(screen.getByTestId("files-explorer")).toHaveTextContent(
      "explorer:project-1::",
    );
    expect(
      screen.getByRole("button", { name: /资源管理器|Explorer/ }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /搜索|Search/ }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", {
        name: /刷新工作区文件|Refresh workspace files/,
      }),
    ).toBeInTheDocument();
  });

  it("forces explorer and forwards a file request from chat", () => {
    renderPanel({
      taskId: "task-1",
      fileRequest: { path: "src/lib.ts", requestId: 1, line: 8 },
    });

    expect(screen.getByTestId("files-explorer")).toHaveTextContent(
      "explorer:project-1:task-1:src/lib.ts:8",
    );
  });

  it("opens a project-scoped file request without a task", () => {
    renderPanel({
      fileRequest: { path: "README.md", requestId: 2, line: 1 },
    });

    expect(screen.getByTestId("files-explorer")).toHaveTextContent(
      "explorer:project-1::README.md:1",
    );
  });

  it("returns from Specs to Explorer for an unresolved artifact request", async () => {
    const user = userEvent.setup();
    const view = renderPanel({ taskId: "task-1" });
    await user.click(screen.getByRole("button", { name: "Specs" }));
    expect(screen.getByTestId("specs-content")).toBeInTheDocument();

    view.rerender(
      <QueryClientProvider client={new QueryClient()}>
        <PlatformProvider adapter={createStubPlatform()}>
          <AppI18nProvider>
            <TooltipProvider>
              <WorkspaceReviewFilesPanel
                projectId="project-1"
                taskId="task-1"
                artifactRequest={{ path: "install", requestId: 1 }}
              />
            </TooltipProvider>
          </AppI18nProvider>
        </PlatformProvider>
      </QueryClientProvider>,
    );
    expect(screen.getByTestId("files-explorer")).toHaveTextContent("install");
  });
});
