import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  QueryClient,
  QueryClientProvider,
  onlineManager,
} from "@tanstack/react-query";
import type {
  GetWorkspaceDiffResponse,
  GetWorkspaceStatusResponse,
} from "@ora/contracts";
import { describe, expect, it, vi } from "vitest";
import { ContractsClientContext } from "../../contracts-client-context";
import { AppI18nProvider } from "../../i18n/i18n";
import "../../i18n/i18n-instance";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { diffKeys } from "../../state/data/diff";
import { workspaceStatusKeys } from "../../state/data/workspace-status";
import { createAppQueryClient } from "../../state/query-client";
import { TaskDiffView } from "./task-diff-view";

const DIFF: GetWorkspaceDiffResponse = {
  kind: "patch" as const,
  contextLimited: false,
  baseCommitId: "base",
  headCommitId: "head",
  patch:
    "diff --git a/a.ts b/a.ts\nnew file mode 100644\nindex 0000000..1111111\n--- /dev/null\n+++ b/a.ts\n@@ -0,0 +1,1 @@\n+hello\n",
};
const STAGED: GetWorkspaceStatusResponse = {
  entries: [{ path: "a.ts", isStaged: true, isUntracked: false }],
};

/** Keeps old and replacement transport responses independently controllable. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

/** Keeps B actively observed so an accidentally broadened refresh is observable. */
function mountReviews(
  handlers: TestHandlers,
  queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  }),
) {
  const mounted = render(
    <QueryClientProvider client={queryClient}>
      <ContractsClientContext.Provider
        value={createTestClient({
          // The diff view waits for the workspace list to confirm Git before reading; an app
          // query client retries a missing handler long enough to outlast these tests.
          listWorkspaces: () => ({
            workspaces: ["A", "B"].map((id) => ({
              id,
              projectId: "project",
              kind: "isolated" as const,
              lifecycle: "active" as const,
              directory: {
                kind: "present" as const,
                git: "repository" as const,
              },
            })),
          }),
          ...handlers,
        })}
      >
        <AppI18nProvider>
          {["A", "B"].map((workspaceId) => (
            <div key={workspaceId} data-testid={workspaceId}>
              <TaskDiffView
                workspaceId={workspaceId}
                hasBaseline
                viewType="unified"
                fileTreeOpen
                onFileTreeOpenChange={() => undefined}
              />
            </div>
          ))}
        </AppI18nProvider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>,
  );
  return {
    unmount: mounted.unmount,
    queryClient,
    a: within(screen.getByTestId("A")),
    b: within(screen.getByTestId("B")),
  };
}

describe("workspace review refresh races", () => {
  it.each(["success", "diff failure", "status failure"])(
    "keeps staging pending through three overlapping rounds: %s",
    async (outcome) => {
      const diffs = Array.from({ length: 3 }, () =>
        deferred<GetWorkspaceDiffResponse>(),
      );
      const statuses = Array.from({ length: 3 }, () =>
        deferred<GetWorkspaceStatusResponse>(),
      );
      let diffCalls = 0;
      let statusCalls = 0;
      const bDiff = vi.fn(() => DIFF);
      const bStatus = vi.fn(() => ({ entries: [] }));
      const { queryClient, a, b } = mountReviews({
        getWorkspaceDiff: ({ workspaceId }) =>
          workspaceId === "B"
            ? bDiff()
            : diffCalls++ === 0
              ? DIFF
              : diffs[diffCalls - 2]!.promise,
        getWorkspaceStatus: ({ workspaceId }) =>
          workspaceId === "B"
            ? bStatus()
            : statusCalls++ === 0
              ? { entries: [] }
              : statuses[statusCalls - 2]!.promise,
        stageWorkspaceChanges: () => ({ stagedPaths: ["a.ts"] }),
      });
      await a.findByRole("button", { name: "暂存 a.ts" });
      await b.findByRole("button", { name: "暂存 a.ts" });
      const bBefore = [
        queryClient.getQueryState(diffKeys.workspaceDiff("B", "branch")),
        queryClient.getQueryState(workspaceStatusKeys.workspaceStatus("B")),
      ];
      const user = userEvent.setup();
      await user.click(a.getByRole("button", { name: "暂存 a.ts" }));
      await waitFor(() => expect([diffCalls, statusCalls]).toEqual([2, 2]));
      for (const count of [3, 4]) {
        await user.click(a.getByRole("button", { name: /刷新/ }));
        await waitFor(() =>
          expect([diffCalls, statusCalls]).toEqual([count, count]),
        );
      }
      expect(queryClient.isMutating()).toBe(1);
      expect(a.getByRole("button", { name: "暂存 a.ts" })).toBeDisabled();
      const failure = new Error("latest refresh failed");
      await act(async () => {
        if (outcome === "diff failure") diffs[2]!.reject(failure);
        else diffs[2]!.resolve({ ...DIFF, headCommitId: "latest" });
        await queryClient
          .getQueryCache()
          .find({ queryKey: diffKeys.workspaceDiff("A", "branch") })!
          .promise?.catch(() => undefined);
      });
      expect(queryClient.isMutating()).toBe(1);
      await act(async () => {
        if (outcome === "status failure") statuses[2]!.reject(failure);
        else statuses[2]!.resolve(STAGED);
        await queryClient
          .getQueryCache()
          .find({ queryKey: workspaceStatusKeys.workspaceStatus("A") })!
          .promise?.catch(() => undefined);
      });
      await waitFor(() => expect(queryClient.isMutating()).toBe(0));
      // Transports may ignore cancellation; late successes and failures must not
      // replace the latest cache or release a different workspace's waiters.
      await act(async () => {
        diffs[0]!.resolve(DIFF);
        statuses[0]!.resolve({ entries: [] });
        diffs[1]!.reject(new Error("obsolete diff failed"));
        statuses[1]!.reject(new Error("obsolete status failed"));
        await Promise.allSettled(
          [...diffs, ...statuses].map((response) => response.promise),
        );
      });
      expect(
        queryClient.getQueryState(diffKeys.workspaceDiff("A", "branch"))?.error,
      ).toBe(outcome === "diff failure" ? failure : null);
      expect(
        queryClient.getQueryState(workspaceStatusKeys.workspaceStatus("A"))
          ?.error,
      ).toBe(outcome === "status failure" ? failure : null);
      if (outcome !== "status failure")
        expect(
          queryClient.getQueryData(workspaceStatusKeys.workspaceStatus("A")),
        ).toEqual(STAGED);
      if (outcome === "success")
        expect(a.getByRole("button", { name: "取消暂存 a.ts" })).toBeEnabled();
      expect([bDiff.mock.calls.length, bStatus.mock.calls.length]).toEqual([
        1, 1,
      ]);
      expect([
        queryClient.getQueryState(diffKeys.workspaceDiff("B", "branch")),
        queryClient.getQueryState(workspaceStatusKeys.workspaceStatus("B")),
      ]).toEqual(bBefore);
    },
  );

  it.each(["old first", "old last"])(
    "replaces an uncached pre-write status request: %s",
    async (order) => {
      const oldStatus = deferred<GetWorkspaceStatusResponse>();
      const newStatus = deferred<GetWorkspaceStatusResponse>();
      let calls = 0;
      const { queryClient, a } = mountReviews({
        getWorkspaceDiff: () => DIFF,
        getWorkspaceStatus: ({ workspaceId }) =>
          workspaceId === "B"
            ? { entries: [] }
            : ++calls === 1
              ? oldStatus.promise
              : newStatus.promise,
        stageWorkspaceChanges: () => ({ stagedPaths: ["a.ts"] }),
      });
      await userEvent
        .setup()
        .click(await a.findByRole("button", { name: "暂存 a.ts" }));
      await waitFor(() => expect(calls).toBe(2));
      if (order === "old first")
        await act(async () => {
          oldStatus.resolve({ entries: [] });
          await oldStatus.promise;
        });
      expect(queryClient.isMutating()).toBe(1);
      expect(a.getByRole("button", { name: "暂存 a.ts" })).toBeDisabled();
      await act(async () => {
        newStatus.resolve(STAGED);
        await newStatus.promise;
      });
      await waitFor(() => expect(queryClient.isMutating()).toBe(0));
      if (order === "old last")
        await act(async () => {
          oldStatus.resolve({ entries: [] });
          await oldStatus.promise;
        });
      expect(a.getByRole("button", { name: "取消暂存 a.ts" })).toBeEnabled();
      expect(
        queryClient.getQueryData(workspaceStatusKeys.workspaceStatus("A")),
      ).toEqual(STAGED);
    },
  );
  it.each(["success", "diff failure", "status failure", "overlap"])(
    "waits for offline refreshes to resume and settle: %s",
    async (outcome) => {
      const wasOnline = onlineManager.isOnline();
      const write = deferred<{ stagedPaths: string[] }>();
      const diff = deferred<GetWorkspaceDiffResponse>();
      const status = deferred<GetWorkspaceStatusResponse>();
      const diffCalls: string[] = [];
      const statusCalls: string[] = [];
      let changed = false;
      const { queryClient, a, b, unmount } = mountReviews(
        {
          getWorkspaceDiff: ({ workspaceId }) => {
            diffCalls.push(workspaceId);
            return workspaceId === "A" && changed ? diff.promise : DIFF;
          },
          getWorkspaceStatus: ({ workspaceId }) => {
            statusCalls.push(workspaceId);
            return workspaceId === "A" && changed
              ? status.promise
              : { entries: [] };
          },
          stageWorkspaceChanges: () => write.promise,
        },
        createAppQueryClient(),
      );
      const aDiffKey = diffKeys.workspaceDiff("A", "branch");
      const aStatusKey = workspaceStatusKeys.workspaceStatus("A");
      try {
        await a.findByRole("button", { name: "暂存 a.ts" });
        await b.findByRole("button", { name: "暂存 a.ts" });
        const bBefore = [
          queryClient.getQueryState(diffKeys.workspaceDiff("B", "branch")),
          queryClient.getQueryState(workspaceStatusKeys.workspaceStatus("B")),
        ];
        const user = userEvent.setup();
        await user.click(a.getByRole("button", { name: "暂存 a.ts" }));
        await act(async () => {
          onlineManager.setOnline(false);
          changed = true;
          write.resolve({ stagedPaths: ["a.ts"] });
          await write.promise;
        });
        await waitFor(() =>
          expect([
            queryClient.getQueryState(aDiffKey)?.fetchStatus,
            queryClient.getQueryState(aStatusKey)?.fetchStatus,
          ]).toEqual(["paused", "paused"]),
        );
        expect(queryClient.isMutating()).toBe(1);
        expect(a.getByRole("button", { name: "暂存 a.ts" })).toBeDisabled();
        if (outcome === "overlap") {
          await user.click(a.getByRole("button", { name: /刷新/ }));
          await user.click(a.getByRole("button", { name: /刷新/ }));
          expect(queryClient.isMutating()).toBe(1);
          expect(a.getByRole("button", { name: "暂存 a.ts" })).toBeDisabled();
        }
        expect([diffCalls, statusCalls]).toEqual([
          ["A", "B"],
          ["A", "B"],
        ]);
        await act(async () => {
          onlineManager.setOnline(true);
        });
        await waitFor(() =>
          expect([diffCalls, statusCalls]).toEqual([
            ["A", "B", "A"],
            ["A", "B", "A"],
          ]),
        );
        expect(queryClient.isMutating()).toBe(1);
        const failure = new Error("resumed review failed");
        await act(async () => {
          if (outcome === "diff failure") diff.reject(failure);
          else diff.resolve(DIFF);
          // Includes the production retry policy before the terminal outcome.
          await queryClient
            .getQueryCache()
            .find({ queryKey: aDiffKey })!
            .promise?.catch(() => undefined);
        });
        expect(queryClient.isMutating()).toBe(1);
        await act(async () => {
          if (outcome === "status failure") status.reject(failure);
          else status.resolve(STAGED);
          await queryClient
            .getQueryCache()
            .find({ queryKey: aStatusKey })!
            .promise?.catch(() => undefined);
        });
        await waitFor(() => expect(queryClient.isMutating()).toBe(0));
        expect([
          queryClient.getQueryState(aDiffKey)?.fetchStatus,
          queryClient.getQueryState(aStatusKey)?.fetchStatus,
        ]).toEqual(["idle", "idle"]);
        expect(queryClient.getQueryState(aDiffKey)?.error).toBe(
          outcome === "diff failure" ? failure : null,
        );
        expect(queryClient.getQueryState(aStatusKey)?.error).toBe(
          outcome === "status failure" ? failure : null,
        );
        if (outcome === "diff failure")
          expect(a.getByText("Ora 返回了无法识别的响应。")).toBeInTheDocument();
        else if (outcome === "status failure")
          expect(a.getByRole("alert")).toHaveTextContent(
            "Ora 返回了无法识别的响应。",
          );
        else
          expect(
            a.getByRole("button", { name: "取消暂存 a.ts" }),
          ).toBeEnabled();
        expect([
          diffCalls.filter((id) => id === "B"),
          statusCalls.filter((id) => id === "B"),
        ]).toEqual([["B"], ["B"]]);
        expect([
          queryClient.getQueryState(diffKeys.workspaceDiff("B", "branch")),
          queryClient.getQueryState(workspaceStatusKeys.workspaceStatus("B")),
        ]).toEqual(bBefore);
      } finally {
        unmount();
        await act(async () => {
          queryClient.clear();
          write.resolve({ stagedPaths: ["a.ts"] });
          diff.resolve(DIFF);
          status.resolve(STAGED);
          onlineManager.setOnline(wasOnline);
        });
      }
    },
  );
});
