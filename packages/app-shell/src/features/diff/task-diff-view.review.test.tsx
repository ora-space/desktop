import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import { RemoteContractError } from "@ora/contracts";
import type {
  GetWorkspaceDiffResponse,
  GetWorkspaceStatusResponse,
} from "@ora/contracts";
import { ContractsClientContext } from "../../contracts-client-context";
import { AppI18nProvider } from "../../i18n/i18n";
import "../../i18n/i18n-instance";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { TaskDiffView } from "./task-diff-view";

const DIFF: GetWorkspaceDiffResponse = {
  kind: "patch" as const,
  contextLimited: false,
  baseCommitId: "base",
  headCommitId: "head",
  patch:
    "diff --git a/a.ts b/a.ts\nnew file mode 100644\nindex 0000000..1111111\n--- /dev/null\n+++ b/a.ts\n@@ -0,0 +1,1 @@\n+hello\n",
};

/** Lets each test release transport responses at the boundary it asserts. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

/** Runs real Changes queries and mutations through explicitly typed transport handlers. */
function mountReview(handlers: TestHandlers) {
  const client = createTestClient(handlers);
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  const view = (workspaceId: string) => (
    <QueryClientProvider client={queryClient}>
      <ContractsClientContext.Provider value={client}>
        <AppI18nProvider>
          <TaskDiffView
            workspaceId={workspaceId}
            hasBaseline
            viewType="unified"
            fileTreeOpen
            onFileTreeOpenChange={() => undefined}
          />
        </AppI18nProvider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>
  );
  return { ...render(view("A")), view, queryClient };
}

describe("workspace review refresh", () => {
  it("manual refresh reloads status as well as the patch", async () => {
    const status = vi.fn(async () => ({ entries: [] }));
    mountReview({
      getWorkspaceDiff: async () => DIFF,
      getWorkspaceStatus: status,
    });
    await screen.findByRole("button", { name: "暂存 a.ts" });
    await userEvent.setup().click(screen.getByRole("button", { name: /刷新/ }));
    await waitFor(() => expect(status).toHaveBeenCalledTimes(2));
  });
  it.each([
    ["stage one", false, "暂存 a.ts", ["a.ts"]],
    ["unstage one", true, "取消暂存 a.ts", ["a.ts"]],
    ["stage all", false, "暂存所有改动", []],
    ["unstage all", true, "取消暂存", ["a.ts"]],
  ] as const)(
    "%s waits for both review responses",
    async (_name, staged, label, paths) => {
      const user = userEvent.setup();
      const diffResponse = deferred<GetWorkspaceDiffResponse>();
      const statusResponse = deferred<GetWorkspaceStatusResponse>();
      let changed = false;
      const write = vi.fn(async () => {
        changed = true;
      });
      const status = vi.fn<NonNullable<TestHandlers["getWorkspaceStatus"]>>(
        () =>
          changed
            ? statusResponse.promise
            : {
                entries: [
                  { path: "a.ts", isStaged: staged, isUntracked: false },
                ],
              },
      );
      const handlers: TestHandlers = {
        getWorkspaceDiff: () => (changed ? diffResponse.promise : DIFF),
        getWorkspaceStatus: status,
        ...(!staged
          ? {
              stageWorkspaceChanges: async (request, options) => {
                await write();
                expect([request, options]).toEqual([
                  { workspaceId: "A", paths: [...paths] },
                  undefined,
                ]);
                return { stagedPaths: ["a.ts"] };
              },
            }
          : {
              unstageWorkspaceChanges: async (request, options) => {
                await write();
                expect([request, options]).toEqual([
                  { workspaceId: "A", paths: [...paths] },
                  undefined,
                ]);
                return { unstagedPaths: ["a.ts"] };
              },
            }),
      };
      mountReview(handlers);
      await screen.findByRole("button", {
        name: staged ? "取消暂存 a.ts" : "暂存 a.ts",
      });
      const all = _name.endsWith("all");
      if (all)
        await user.click(
          screen.getByRole("button", { name: "提交和推送操作" }),
        );
      await user.click(screen.getByRole("button", { name: label }));
      await waitFor(() => expect(write).toHaveBeenCalledOnce());
      expect(status.mock.calls).toEqual([
        [{ workspaceId: "A" }, undefined],
        [{ workspaceId: "A" }, undefined],
      ]);
      expect(screen.getByRole("button", { name: label })).toBeDisabled();
      await act(async () => {
        diffResponse.resolve(DIFF);
        await diffResponse.promise;
      });
      expect(screen.getByRole("button", { name: label })).toBeDisabled();
      await act(async () => {
        statusResponse.resolve({
          entries: [{ path: "a.ts", isStaged: !staged, isUntracked: false }],
        });
        await statusResponse.promise;
      });
      await waitFor(() =>
        expect(
          screen.getByRole("button", {
            name: all ? "暂存所有改动" : staged ? "暂存 a.ts" : "取消暂存 a.ts",
          }),
        ).toBeEnabled(),
      );
      if (!all)
        await user.click(
          screen.getByRole("button", { name: "提交和推送操作" }),
        );
      expect(
        screen.getByText(
          staged ? "没有已暂存的更改" : "将提交已暂存的 1 项更改",
        ),
      ).toBeInTheDocument();
      await user.type(
        screen.getByRole("textbox", { name: "提交说明" }),
        "review changes",
      );
      const commitButton = screen.getByRole("button", { name: "提交" });
      if (staged) expect(commitButton).toBeDisabled();
      else expect(commitButton).toBeEnabled();
    },
  );

  it.each([false, true])(
    "shows a failed write without refreshing (initially staged: %s)",
    async (staged) => {
      const response = deferred<never>();
      const status = vi.fn<NonNullable<TestHandlers["getWorkspaceStatus"]>>(
        () => ({
          entries: [{ path: "a.ts", isStaged: staged, isUntracked: false }],
        }),
      );
      const diff = vi.fn(() => DIFF);
      const write = vi.fn<NonNullable<TestHandlers["stageWorkspaceChanges"]>>(
        () => response.promise,
      );
      const unstage = vi.fn<
        NonNullable<TestHandlers["unstageWorkspaceChanges"]>
      >(() => response.promise);
      const mounted = mountReview({
        getWorkspaceDiff: diff,
        getWorkspaceStatus: status,
        ...(staged
          ? { unstageWorkspaceChanges: unstage }
          : { stageWorkspaceChanges: write }),
      });
      const label = staged ? "取消暂存 a.ts" : "暂存 a.ts";
      await userEvent
        .setup()
        .click(await screen.findByRole("button", { name: label }));
      expect((staged ? unstage : write).mock.calls).toEqual([
        [{ workspaceId: "A", paths: ["a.ts"] }, undefined],
      ]);
      expect(screen.getByRole("button", { name: label })).toBeDisabled();
      await act(async () => {
        response.reject(
          new RemoteContractError(
            {
              code: "internal_error",
              params: {},
              requestId: "review-write-failed",
            },
            null,
          ),
        );
      });
      expect(await screen.findByRole("alert")).toHaveTextContent(
        "review-write-failed",
      );
      await waitFor(() => expect(mounted.queryClient.isMutating()).toBe(0));
      expect(screen.getByRole("button", { name: label })).toBeEnabled();
      expect(status.mock.calls).toEqual([[{ workspaceId: "A" }, undefined]]);
      expect(diff).toHaveBeenCalledOnce();
    },
  );

  it("waits for status even when diff refresh fails, then allows recovery", async () => {
    const diffResponse = deferred<GetWorkspaceDiffResponse>();
    const statusResponse = deferred<GetWorkspaceStatusResponse>();
    let changed = false;
    const mounted = mountReview({
      getWorkspaceDiff: () => (changed ? diffResponse.promise : DIFF),
      getWorkspaceStatus: () =>
        changed ? statusResponse.promise : { entries: [] },
      stageWorkspaceChanges: () => {
        changed = true;
        return { stagedPaths: ["a.ts"] };
      },
    });
    await userEvent
      .setup()
      .click(await screen.findByRole("button", { name: "暂存 a.ts" }));
    await act(async () => {
      diffResponse.reject(new Error("diff refresh failed"));
    });
    expect(
      await screen.findByText("Ora 返回了无法识别的响应。"),
    ).toBeInTheDocument();
    expect(mounted.queryClient.isMutating()).toBe(1);
    await act(async () => {
      statusResponse.resolve({ entries: [] });
      await statusResponse.promise;
    });
    await waitFor(() => expect(mounted.queryClient.isMutating()).toBe(0));
    changed = false;
    await userEvent.setup().click(screen.getByRole("button", { name: "重试" }));
    expect(
      await screen.findByRole("button", { name: "暂存 a.ts" }),
    ).toBeEnabled();
  });

  it("finishes an A write after switching to B without invalidating B", async () => {
    const response = deferred<{ stagedPaths: string[] }>();
    const diff = vi.fn(async () => DIFF);
    const status = vi.fn(async () => ({ entries: [] }));
    const mounted = mountReview({
      getWorkspaceDiff: diff,
      getWorkspaceStatus: status,
      stageWorkspaceChanges: () => response.promise,
    });
    await userEvent
      .setup()
      .click(await screen.findByRole("button", { name: "暂存 a.ts" }));
    mounted.rerender(mounted.view("B"));
    await waitFor(() => expect(status).toHaveBeenCalledTimes(2));
    await act(async () => {
      response.resolve({ stagedPaths: ["a.ts"] });
      await response.promise;
    });
    await waitFor(() => expect(mounted.queryClient.isMutating()).toBe(0));
    expect(status.mock.calls).toHaveLength(2);
    expect(diff.mock.calls).toHaveLength(2);
    mounted.rerender(mounted.view("A"));
    await waitFor(() => expect(status).toHaveBeenCalledTimes(3));
    expect(diff.mock.calls).toHaveLength(3);
  });
  it("presents a failed status refresh and recovers its staging marker on retry", async () => {
    let failed = false;
    const error = new RemoteContractError(
      { code: "internal_error", params: {}, requestId: "review-status-failed" },
      null,
    );
    const mounted = mountReview({
      getWorkspaceDiff: async () => DIFF,
      getWorkspaceStatus: async () => {
        if (failed) throw error;
        return {
          entries: [{ path: "a.ts", isStaged: false, isUntracked: false }],
        };
      },
      stageWorkspaceChanges: () => {
        failed = true;
        return { stagedPaths: ["a.ts"] };
      },
    });
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "暂存 a.ts" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "review-status-failed",
    );
    await waitFor(() => expect(mounted.queryClient.isMutating()).toBe(0));
    failed = false;
    await user.click(screen.getByRole("button", { name: /刷新/ }));
    await waitFor(() =>
      expect(screen.queryByRole("alert")).not.toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: "暂存 a.ts" })).toBeEnabled();
  });

  it("keeps commit pending until diff and status refreshes settle", async () => {
    const diffResponse = deferred<GetWorkspaceDiffResponse>();
    const statusResponse = deferred<GetWorkspaceStatusResponse>();
    let committed = false;
    const commit = vi.fn<NonNullable<TestHandlers["commitWorkspaceChanges"]>>(
      async (request, options) => {
        expect([request, options]).toEqual([
          { workspaceId: "A", message: "review changes" },
          undefined,
        ]);
        committed = true;
        return { commitId: "new-head", summary: "review changes" };
      },
    );
    const mounted = mountReview({
      getWorkspaceDiff: () => (committed ? diffResponse.promise : DIFF),
      getWorkspaceStatus: () =>
        committed
          ? statusResponse.promise
          : { entries: [{ path: "a.ts", isStaged: true, isUntracked: false }] },
      commitWorkspaceChanges: commit,
    });
    const user = userEvent.setup();
    await screen.findByRole("button", { name: "取消暂存 a.ts" });
    await user.click(screen.getByRole("button", { name: "提交和推送操作" }));
    await user.type(
      screen.getByRole("textbox", { name: "提交说明" }),
      "review changes",
    );
    await user.click(screen.getByRole("button", { name: "提交" }));
    await waitFor(() => expect(commit).toHaveBeenCalledOnce());
    expect(screen.getByRole("textbox", { name: "提交说明" })).toBeDisabled();
    await act(async () => {
      statusResponse.resolve({ entries: [] });
      await statusResponse.promise;
    });
    expect(mounted.queryClient.isMutating()).toBe(1);
    await act(async () => {
      diffResponse.resolve({ ...DIFF, headCommitId: "new-head" });
      await diffResponse.promise;
    });
    await waitFor(() => expect(mounted.queryClient.isMutating()).toBe(0));
    await waitFor(() =>
      expect(
        screen.queryByRole("textbox", { name: "提交说明" }),
      ).not.toBeInTheDocument(),
    );
  });
});
