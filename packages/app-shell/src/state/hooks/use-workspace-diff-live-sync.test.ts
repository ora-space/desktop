import { act, renderHook } from "@testing-library/react";
import {
  QueryClient,
  QueryClientProvider,
  QueryObserver,
} from "@tanstack/react-query";
import { createElement, type ReactNode } from "react";
import {
  createChatStore,
  type ChatStore,
  type SessionConversation,
} from "@ora/chat";
import type { Session } from "@ora/contracts";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createTestClient } from "../../test/contracts-transport";
import "../../i18n/i18n-instance";
import { diffKeys } from "../data/diff";
import { workspaceStatusKeys } from "../data/workspace-status";
import { useWorkspaceDiffLiveSync } from "./use-workspace-diff-live-sync";
import { AGENT_REF } from "../../test/agent-identity";

const SESSION: Session = {
  id: "session-1",
  workspaceId: "workspace-1",
  agentRef: AGENT_REF.codeagentcli,
  status: "running",
  title: null,
  historyState: { type: "writable" },
  mcpSelection: { mode: "automatic" },
};

/** Builds one conversation state with just enough lifecycle data for diff syncing. */
function conversation(
  isResponding: boolean,
  toolStatus?: "in_progress" | "completed" | "failed",
): SessionConversation {
  return {
    configOptions: [],
    modelChanges: [],
    historyNotices: [],
    turns:
      toolStatus === undefined
        ? []
        : [
            {
              id: "turn-1",
              userMessage: {
                kind: "message",
                id: "message-1",
                role: "user",
                content: "change it",
                createdAt: 1,
              },
              items: [
                {
                  kind: "toolCall",
                  id: "tool-1",
                  title: "Edit file",
                  toolKind: "edit",
                  status: toolStatus,
                  content: [],
                  locations: [],
                  createdAt: 2,
                  updatedAt: 3,
                },
              ],
              status: isResponding ? "streaming" : "completed",
              stopReason: null,
              error: null,
              createdAt: 1,
            },
          ],
    availableCommands: [],
    isLoaded: true,
    isLoading: false,
    isResponding,
    sessionTitle: null,
    sessionUpdatedAt: null,
    pendingPermissions: [],
    usage: {
      context: { status: "hidden" },
      lastTurnTokens: { status: "none" },
    },
    error: null,
  };
}

/** Creates an isolated chat store whose state tests can advance directly. */
function makeChatStore(): ChatStore {
  return createChatStore(createTestClient({}).session);
}

/** Provides the query cache observed by the live-sync hook. */
function wrapper(queryClient: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return createElement(
      QueryClientProvider,
      { client: queryClient },
      children,
    );
  };
}

beforeEach(() => vi.useFakeTimers());
afterEach(() => vi.useRealTimers());

describe("useWorkspaceDiffLiveSync", () => {
  it("invalidates the aggregate workspace diff after a live file change completes", async () => {
    const chatStore = makeChatStore();
    chatStore.setState({
      conversations: { [SESSION.id]: conversation(true, "in_progress") },
    });
    const queryClient = new QueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    renderHook(() => useWorkspaceDiffLiveSync(chatStore, [SESSION]), {
      wrapper: wrapper(queryClient),
    });

    await act(async () => {
      chatStore.setState({
        conversations: { [SESSION.id]: conversation(true, "completed") },
      });
      await vi.advanceTimersByTimeAsync(400);
    });

    expect(invalidate).toHaveBeenCalledTimes(2);
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: diffKeys.workspaceDiffs(SESSION.workspaceId),
    });
    expect(invalidate).toHaveBeenCalledWith({
      queryKey: workspaceStatusKeys.workspaceStatus(SESSION.workspaceId),
    });
  });

  it("coalesces the file-change and turn-completed refreshes", async () => {
    const chatStore = makeChatStore();
    chatStore.setState({
      conversations: { [SESSION.id]: conversation(true, "in_progress") },
    });
    const queryClient = new QueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    renderHook(() => useWorkspaceDiffLiveSync(chatStore, [SESSION]), {
      wrapper: wrapper(queryClient),
    });

    await act(async () => {
      chatStore.setState({
        conversations: { [SESSION.id]: conversation(true, "completed") },
      });
      chatStore.setState({
        conversations: { [SESSION.id]: conversation(false, "completed") },
      });
      await vi.advanceTimersByTimeAsync(400);
    });

    expect(invalidate).toHaveBeenCalledTimes(2);
  });

  it("does not treat replayed completed tools as live changes", () => {
    const chatStore = makeChatStore();
    const queryClient = new QueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    renderHook(() => useWorkspaceDiffLiveSync(chatStore, [SESSION]), {
      wrapper: wrapper(queryClient),
    });

    act(() => {
      chatStore.setState({
        conversations: { [SESSION.id]: conversation(false, "completed") },
      });
      vi.advanceTimersByTime(400);
    });

    expect(invalidate).not.toHaveBeenCalled();
  });
  it("refreshes live A queries after a turn without blocking the event or touching B", async () => {
    const chatStore = makeChatStore();
    chatStore.setState({ conversations: { [SESSION.id]: conversation(true) } });
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false, staleTime: Infinity } },
    });
    let release!: () => void;
    const response = new Promise<void>((resolve) => {
      release = resolve;
    });
    const diff = { baseCommitId: "base", headCommitId: "head", patch: "old" };
    const status = { entries: [] };
    const getDiff = vi.fn(async () => {
      await response;
      return { ...diff, patch: "new" };
    });
    const getStatus = vi.fn(async () => {
      await response;
      return {
        entries: [{ path: "a.ts", isStaged: true, isUntracked: false }],
      };
    });
    const client = createTestClient({
      getWorkspaceDiff: getDiff,
      getWorkspaceStatus: getStatus,
    });
    const stops: (() => void)[] = [];
    for (const workspaceId of [SESSION.workspaceId, "B"]) {
      const diffKey = diffKeys.workspaceDiff(workspaceId, "branch");
      const statusKey = workspaceStatusKeys.workspaceStatus(workspaceId);
      queryClient.setQueryData(diffKey, diff);
      queryClient.setQueryData(statusKey, status);
      stops.push(
        new QueryObserver(queryClient, {
          queryKey: diffKey,
          queryFn: () =>
            client.workspace.getDiff({ workspaceId, scope: "branch" }),
        }).subscribe(() => undefined),
      );
      stops.push(
        new QueryObserver(queryClient, {
          queryKey: statusKey,
          queryFn: () => client.workspace.getStatus({ workspaceId }),
        }).subscribe(() => undefined),
      );
    }
    const mounted = renderHook(
      () => useWorkspaceDiffLiveSync(chatStore, [SESSION]),
      { wrapper: wrapper(queryClient) },
    );
    await act(async () => {
      chatStore.setState({
        conversations: { [SESSION.id]: conversation(false) },
      });
      await vi.advanceTimersByTimeAsync(400);
    });
    expect(chatStore.getState().conversations[SESSION.id]?.isResponding).toBe(
      false,
    );
    expect(queryClient.isFetching()).toBe(2);
    expect(getDiff.mock.calls).toEqual([
      [{ workspaceId: SESSION.workspaceId, scope: "branch" }, undefined],
    ]);
    expect(getStatus.mock.calls).toEqual([
      [{ workspaceId: SESSION.workspaceId }, undefined],
    ]);
    await act(async () => {
      release();
      await Promise.all(
        queryClient
          .getQueryCache()
          .findAll()
          .map((query) => query.promise),
      );
    });
    expect(
      queryClient.getQueryData(
        diffKeys.workspaceDiff(SESSION.workspaceId, "branch"),
      ),
    ).toEqual({ ...diff, patch: "new" });
    expect(
      queryClient.getQueryData(
        workspaceStatusKeys.workspaceStatus(SESSION.workspaceId),
      ),
    ).toEqual({
      entries: [{ path: "a.ts", isStaged: true, isUntracked: false }],
    });
    expect(
      queryClient.getQueryData(diffKeys.workspaceDiff("B", "branch")),
    ).toEqual(diff);
    expect(
      queryClient.getQueryData(workspaceStatusKeys.workspaceStatus("B")),
    ).toEqual(status);
    mounted.unmount();
    stops.forEach((stop) => stop());
    queryClient.clear();
  });

  it("cancels scheduled refreshes on unmount", () => {
    const chatStore = makeChatStore();
    chatStore.setState({ conversations: { [SESSION.id]: conversation(true) } });
    const queryClient = new QueryClient();
    const invalidate = vi.spyOn(queryClient, "invalidateQueries");
    const mounted = renderHook(
      () => useWorkspaceDiffLiveSync(chatStore, [SESSION]),
      { wrapper: wrapper(queryClient) },
    );
    act(() =>
      chatStore.setState({
        conversations: { [SESSION.id]: conversation(false) },
      }),
    );
    mounted.unmount();
    act(() => vi.advanceTimersByTime(400));
    expect(invalidate).not.toHaveBeenCalled();
    queryClient.clear();
  });
});
