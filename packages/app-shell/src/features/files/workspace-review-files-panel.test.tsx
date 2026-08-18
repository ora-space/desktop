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
  }: {
    surface: "explorer" | "search";
    fileRequest?: { path: string; requestId: number; line?: number };
  }) => (
    <div data-testid="files-explorer">
      {surface}:{fileRequest?.path ?? ""}:{fileRequest?.line ?? ""}
    </div>
  ),
}));

function renderPanel(props: {
  projectId?: string;
  taskId?: string;
  fileRequest?: { path: string; requestId: number; line?: number };
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

  it("opens project files directly on specs and hides explorer/search toggles", () => {
    renderPanel({});

    expect(screen.getByTestId("specs-content")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /浏览|Explorer/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /搜索|Search/ }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /刷新 Specs|Refresh Specs/ }),
    ).toBeInTheDocument();
  });

  it("forces explorer and forwards a file request from chat", () => {
    renderPanel({
      taskId: "task-1",
      fileRequest: { path: "src/lib.ts", requestId: 1, line: 8 },
    });

    expect(screen.getByTestId("files-explorer")).toHaveTextContent(
      "explorer:src/lib.ts:8",
    );
  });
});
