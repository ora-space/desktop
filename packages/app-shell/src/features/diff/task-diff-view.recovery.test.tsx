import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RemoteContractError } from "@ora/contracts";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { ContractsClientContext } from "../../contracts-client-context";
import { AppI18nProvider } from "../../i18n/i18n";
import "../../i18n/i18n-instance";
import { PlatformProvider } from "../../platform";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { createSurfaceTestPlatform } from "../../test/surface-test-platform";
import { WorkspaceReviewLayout } from "../workspace/workspace-review-layout";

vi.mock("../files/workspace-review-files-panel", () => ({
  WorkspaceReviewFilesPanel: ({ toolbar }: { toolbar?: ReactNode }) => (
    <section aria-label="Files panel">{toolbar}</section>
  ),
}));

function mountReview(
  workspaceId: string,
  getWorkspaceDiff: NonNullable<TestHandlers["getWorkspaceDiff"]>,
  retry = false as false | number,
) {
  const projectId = `project-${workspaceId}`;
  const client = createTestClient({
    getWorkspaceDiff,
    getWorkspaceStatus: async () => ({ entries: [] }),
    listWorkspaces: async () => ({
      workspaces: [
        {
          id: workspaceId,
          projectId,
          kind: "main",
          lifecycle: "active",
          directory: { kind: "present", git: "repository" },
        },
      ],
    }),
  });
  const queries = new QueryClient({
    defaultOptions: { queries: { retry } },
  });
  const host = createSurfaceTestPlatform({ embedded: false });
  render(
    <QueryClientProvider client={queries}>
      <ContractsClientContext.Provider value={client}>
        <AppI18nProvider>
          <PlatformProvider adapter={host.platform}>
            <WorkspaceReviewLayout
              context={{ kind: "project", projectId, workspaceId }}
            >
              <div>conversation</div>
            </WorkspaceReviewLayout>
          </PlatformProvider>
        </AppI18nProvider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>,
  );
}

const tooLarge = () =>
  new RemoteContractError(
    {
      code: "workspace_diff_too_large",
      params: {},
      requestId: "550e8400-e29b-41d4-a716-446655440000",
    },
    null,
  );

const patch = {
  baseCommitId: null,
  headCommitId: "head",
  patch:
    "diff --git a/small.txt b/small.txt\nnew file mode 100644\n--- /dev/null\n+++ b/small.txt\n@@ -0,0 +1 @@\n+small change\n",
};

describe("Git diff recovery in the workspace review panel", () => {
  it("lets the user switch to Files from an expanded diff error and retry Changes", async () => {
    let fail = true;
    mountReview("failed", async () => {
      if (fail) throw new Error("test transport failure");
      return patch;
    });
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "变更" }));
    await screen.findByText("无法加载变更");

    await user.click(screen.getByRole("button", { name: "宽屏展开变更面板" }));
    const expanded = screen.getByRole("region", { name: "宽屏变更面板" });
    expect(expanded).toContainElement(await screen.findByText("无法加载变更"));
    await user.click(screen.getByRole("button", { name: "文件" }));
    expect(
      screen.getByRole("region", { name: "Files panel" }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "变更" }));
    await screen.findByText("无法加载变更");
    fail = false;
    await user.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByText("small change")).toBeInTheDocument();
  });

  it("renders a compact aggregate in the normal Changes view", async () => {
    const getWorkspaceDiff = vi.fn(async () => ({
      ...patch,
      contextLimited: true,
    }));
    mountReview("compact", getWorkspaceDiff);
    await userEvent.setup().click(screen.getByRole("button", { name: "变更" }));
    expect(await screen.findByText("small change")).toBeInTheDocument();
    expect(screen.getByText("仅显示变更附近的上下文。")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "返回变更视图" })).toBeNull();
    expect(getWorkspaceDiff).toHaveBeenCalledTimes(1);
  });

  it("lists oversized changes and isolates a file that exceeds the preview limit", async () => {
    const getWorkspaceDiff = vi.fn<
      NonNullable<TestHandlers["getWorkspaceDiff"]>
    >(async ({ selection }) => {
      if (selection === undefined) throw tooLarge();
      if (selection.kind === "files") {
        return {
          ...patch,
          patch: "",
          files: [
            { path: "huge.txt", oldPath: null, status: "M" },
            { path: "small.txt", oldPath: null, status: "A" },
          ],
        };
      }
      if (selection.path === "huge.txt") throw tooLarge();
      return patch;
    });
    mountReview("large", getWorkspaceDiff, 1);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "变更" }));
    const viewByFile = await screen.findByRole("button", {
      name: "按文件查看",
    });
    expect(
      getWorkspaceDiff.mock.calls.map(([request]) => request.selection),
    ).toEqual([undefined]);
    expect(screen.queryByRole("group", { name: "变更文件导航" })).toBeNull();
    await user.click(viewByFile);
    expect(screen.getByRole("button", { name: "返回变更视图" })).toHaveFocus();
    expect(await screen.findByText("该文件的 Diff 过大")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "文件" })).toBeInTheDocument();
    expect(
      getWorkspaceDiff.mock.calls.map(([request]) => request.selection),
    ).toEqual([
      undefined,
      { kind: "files" },
      { kind: "file", path: "huge.txt", oldPath: null },
    ]);
    await user.click(screen.getByRole("button", { name: /small\.txt/ }));
    expect(await screen.findByText("small change")).toBeInTheDocument();
    expect(screen.queryByText("该文件的 Diff 过大")).toBeNull();
    await user.click(screen.getByRole("button", { name: "返回变更视图" }));
    expect(screen.queryByRole("group", { name: "变更文件导航" })).toBeNull();
    expect(
      screen.getByRole("button", { name: "按文件查看" }),
    ).toBeInTheDocument();
  });

  it("refreshes the list when the file preview belongs to a different HEAD", async () => {
    let listHead = "old";
    mountReview("moved-head", async ({ selection }) => {
      if (selection === undefined) throw tooLarge();
      if (selection.kind === "files") {
        return {
          ...patch,
          headCommitId: listHead,
          patch: "",
          files: [{ path: "small.txt", oldPath: null, status: "A" }],
        };
      }
      return patch;
    });
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "变更" }));
    await user.click(await screen.findByRole("button", { name: "按文件查看" }));
    expect(await screen.findByText("Git 比较已更新")).toBeInTheDocument();
    expect(screen.queryByText("small change")).toBeNull();
    listHead = patch.headCommitId;
    await user.click(screen.getByRole("button", { name: "重试" }));
    expect(await screen.findByText("small change")).toBeInTheDocument();
  });
});
