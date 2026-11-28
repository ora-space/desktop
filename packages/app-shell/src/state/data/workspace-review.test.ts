import { QueryClient, QueryObserver } from "@tanstack/react-query";
import type {
  GetWorkspaceDiffResponse,
  GetWorkspaceStatusResponse,
  WorkspaceDiffScope,
} from "@ora/contracts";
import { describe, expect, it, vi } from "vitest";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { diffKeys } from "./diff";
import { workspaceStatusKeys } from "./workspace-status";
import { refreshWorkspaceReview } from "./workspace-review";

const DIFF: GetWorkspaceDiffResponse = {
  kind: "patch" as const,
  contextLimited: false,
  baseCommitId: "base",
  headCommitId: "head",
  patch: "old",
};
const STATUS: GetWorkspaceStatusResponse = { entries: [] };

/** Separates refresh completion from transport response order. */
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

describe("refreshWorkspaceReview", () => {
  it.each(["success", "status failure"])(
    "waits for all active scopes with %s and isolates B",
    async (outcome) => {
      const queryClient = new QueryClient({
        defaultOptions: { queries: { retry: false, staleTime: Infinity } },
      });
      const diffResponse = deferred<GetWorkspaceDiffResponse>();
      const statusResponse = deferred<GetWorkspaceStatusResponse>();
      const getDiff = vi.fn(() => diffResponse.promise);
      const getStatus = vi.fn(() => statusResponse.promise);
      const client = createTestClient({
        getWorkspaceDiff: getDiff,
        getWorkspaceStatus: getStatus,
      });
      const unsubscribe: (() => void)[] = [];
      const scopes: WorkspaceDiffScope[] = [
        "branch",
        "unstaged",
        "staged",
        "committed",
      ];
      for (const workspaceId of ["A", "B"]) {
        for (const scope of scopes) {
          const queryKey = diffKeys.workspaceDiff(workspaceId, scope);
          queryClient.setQueryData(queryKey, DIFF);
          unsubscribe.push(
            new QueryObserver(queryClient, {
              queryKey,
              queryFn: () =>
                client.workspace.getDiff({
                  workspaceId,
                  scope,
                  selection: { kind: "patch" },
                }),
            }).subscribe(() => undefined),
          );
        }
        const queryKey = workspaceStatusKeys.workspaceStatus(workspaceId);
        queryClient.setQueryData(queryKey, STATUS);
        unsubscribe.push(
          new QueryObserver(queryClient, {
            queryKey,
            queryFn: () => client.workspace.getStatus({ workspaceId }),
          }).subscribe(() => undefined),
        );
      }
      const bBefore = queryClient
        .getQueryCache()
        .findAll()
        .filter((query) => query.queryKey[1] === "B")
        .map((query) => query.state);
      const done = vi.fn();
      const refresh = refreshWorkspaceReview(queryClient, "A").then(done);
      await vi.waitFor(() =>
        expect(getDiff).toHaveBeenCalledTimes(scopes.length),
      );
      expect(getDiff.mock.calls).toEqual(
        scopes.map((scope) => [
          { workspaceId: "A", scope, selection: { kind: "patch" } },
          undefined,
        ]),
      );
      expect(getStatus.mock.calls).toEqual([[{ workspaceId: "A" }, undefined]]);
      const failure = new Error("status unavailable");
      if (outcome === "success") statusResponse.resolve(STATUS);
      else statusResponse.reject(failure);
      // Wait on the actual query boundary, rather than a timer or a promise flush.
      await queryClient
        .getQueryCache()
        .find({ queryKey: workspaceStatusKeys.workspaceStatus("A") })!
        .promise?.catch(() => undefined);
      expect(done).not.toHaveBeenCalled();
      diffResponse.resolve({ ...DIFF, patch: "new" });
      await refresh;
      expect(done).toHaveBeenCalledOnce();
      for (const scope of scopes) {
        expect(
          queryClient.getQueryData(diffKeys.workspaceDiff("A", scope)),
        ).toEqual({ ...DIFF, patch: "new" });
      }
      expect(
        queryClient.getQueryState(workspaceStatusKeys.workspaceStatus("A"))
          ?.error,
      ).toBe(outcome === "success" ? null : failure);
      expect(
        queryClient
          .getQueryCache()
          .findAll()
          .filter((query) => query.queryKey[1] === "B")
          .map((query) => query.state),
      ).toEqual(bBefore);
      unsubscribe.forEach((stop) => stop());
      queryClient.clear();
    },
  );

  it("marks inactive scopes and status stale without fetching", async () => {
    const queryClient = new QueryClient();
    const keys = [
      diffKeys.workspaceDiff("A", "committed"),
      workspaceStatusKeys.workspaceStatus("A"),
    ];
    for (const key of keys) queryClient.setQueryData(key, {});
    await refreshWorkspaceReview(queryClient, "A");
    expect(
      keys.map((key) => queryClient.getQueryState(key)?.isInvalidated),
    ).toEqual([true, true]);
    expect(queryClient.isFetching()).toBe(0);
    queryClient.clear();
  });
  it("shares completion during cancellation and starts a fresh round after failure", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: Infinity } },
    });
    const first = deferred<GetWorkspaceStatusResponse>();
    const next = deferred<GetWorkspaceStatusResponse>();
    const getStatus = vi
      .fn<NonNullable<TestHandlers["getWorkspaceStatus"]>>()
      .mockImplementationOnce(() => first.promise)
      .mockImplementationOnce(() => next.promise);
    const client = createTestClient({ getWorkspaceStatus: getStatus });
    const queryKey = workspaceStatusKeys.workspaceStatus("A");
    queryClient.setQueryData(queryKey, STATUS);
    const stop = new QueryObserver(queryClient, {
      queryKey,
      queryFn: () => client.workspace.getStatus({ workspaceId: "A" }),
    }).subscribe(() => undefined);
    const initial = refreshWorkspaceReview(queryClient, "A");
    const overlapping = [
      refreshWorkspaceReview(queryClient, "A"),
      refreshWorkspaceReview(queryClient, "A"),
    ];
    for (const completion of overlapping) expect(completion).toBe(initial);
    await vi.waitFor(() => expect(getStatus).toHaveBeenCalledOnce());
    first.reject(new Error("refresh failed"));
    await Promise.all([initial, ...overlapping]);
    const renewed = refreshWorkspaceReview(queryClient, "A");
    expect(renewed).not.toBe(initial);
    await vi.waitFor(() => expect(getStatus).toHaveBeenCalledTimes(2));
    next.resolve(STATUS);
    await renewed;
    expect(queryClient.getQueryState(queryKey)?.error).toBeNull();
    stop();
    queryClient.clear();
  });
  it("keeps waiters for the same workspace in different QueryClients independent", async () => {
    const responses = [
      deferred<GetWorkspaceStatusResponse>(),
      deferred<GetWorkspaceStatusResponse>(),
    ];
    const fixtures = responses.map((response) => {
      const queryClient = new QueryClient({
        defaultOptions: { queries: { retry: false, staleTime: Infinity } },
      });
      const getStatus = vi.fn(() => response.promise);
      const client = createTestClient({ getWorkspaceStatus: getStatus });
      const queryKey = workspaceStatusKeys.workspaceStatus("A");
      queryClient.setQueryData(queryKey, STATUS);
      const stop = new QueryObserver(queryClient, {
        queryKey,
        queryFn: () => client.workspace.getStatus({ workspaceId: "A" }),
      }).subscribe(() => undefined);
      const done = vi.fn();
      const completion = refreshWorkspaceReview(queryClient, "A").then(done);
      return { queryClient, getStatus, stop, done, completion };
    });
    await vi.waitFor(() =>
      expect(
        fixtures.map((fixture) => fixture.getStatus.mock.calls.length),
      ).toEqual([1, 1]),
    );
    responses[0]!.resolve(STATUS);
    await fixtures[0]!.completion;
    expect(fixtures[1]!.done).not.toHaveBeenCalled();
    responses[1]!.resolve(STATUS);
    await fixtures[1]!.completion;
    for (const fixture of fixtures) {
      expect(fixture.done).toHaveBeenCalledOnce();
      fixture.stop();
      fixture.queryClient.clear();
    }
  });
});
