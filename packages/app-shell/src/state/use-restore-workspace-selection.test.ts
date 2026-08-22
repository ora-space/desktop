import { act, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Project, Session, Task } from "@ora/contracts";
import { createChatStore } from "@ora/chat";
import { createMockClient, createMockClientState } from "../test/mock-client";
import { renderHookWithClient } from "../test/hook-harness";
import { useRestoreWorkspaceSelection } from "./hooks/use-restore-workspace-selection";
import { useWarmSession } from "./hooks/use-warm-session";
import { useUiStore } from "./stores/ui-store";
import { useWorkspaceSelectionStore } from "./stores/workspace-selection-store";
import { useDraftSessionsStore } from "./stores/draft-sessions-store";
import { EMPTY_WORKSPACE_SELECTION } from "./stores/sanitize-workspace-selection";

const PROJECT: Project = { id: "p1", name: "Ora", rootPath: "/ora" };
const TASK: Task = {
  id: "t1",
  projectId: "p1",
  title: "Refactor",
  workspaceMode: "worktree",
  type: "default",
  workflowRunId: null,
};
const SESSION: Session = {
  id: "s1",
  taskId: "t1",
  agentRef: "ora-space.opencode",
  status: "running",
  title: null,
  historyState: { type: "writable" },
};

beforeEach(() => {
  window.localStorage.clear();
  useDraftSessionsStore.getState().clear();
  useWorkspaceSelectionStore.setState({
    selection: EMPTY_WORKSPACE_SELECTION,
    pendingRestore: null,
    createFocus: null,
  });
  useUiStore.setState({
    expandedProjects: new Set(),
    expandedTasks: new Set(),
    treeExpansionBootstrapped: false,
  });
  vi.restoreAllMocks();
});

describe("useRestoreWorkspaceSelection", () => {
  it("applies a validated session once the tree is settled", async () => {
    useWorkspaceSelectionStore.setState({
      selection: EMPTY_WORKSPACE_SELECTION,
      pendingRestore: {
        projectId: "p1",
        taskId: "t1",
        sessionId: "s1",
        workflowRunId: null,
        draftId: null,
      },
    });
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    state.sessions = [SESSION];
    const client = createMockClient(state);

    renderHookWithClient(
      () =>
        useRestoreWorkspaceSelection({
          projects: state.projects,
          tasks: state.tasks,
          sessions: state.sessions,
          treePending: false,
        }),
      client,
    );

    await waitFor(() => {
      expect(useWorkspaceSelectionStore.getState().selection).toEqual({
        projectId: "p1",
        taskId: "t1",
        sessionId: "s1",
        workflowRunId: null,
        draftId: null,
      });
    });
    expect(useWorkspaceSelectionStore.getState().pendingRestore).toBeNull();
    // Selection restore must not force-expand; expand state is owned by ui-store.
    expect(useUiStore.getState().expandedProjects.has("p1")).toBe(false);
    expect(useUiStore.getState().expandedTasks.has("t1")).toBe(false);
  });

  it("keeps a collapsed project collapsed when restoring its session", async () => {
    useUiStore.setState({
      expandedProjects: new Set(),
      expandedTasks: new Set(),
      treeExpansionBootstrapped: true,
    });
    useWorkspaceSelectionStore.setState({
      selection: EMPTY_WORKSPACE_SELECTION,
      pendingRestore: {
        projectId: "p1",
        taskId: "t1",
        sessionId: "s1",
        workflowRunId: null,
        draftId: null,
      },
    });
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    state.sessions = [SESSION];
    const client = createMockClient(state);

    renderHookWithClient(
      () =>
        useRestoreWorkspaceSelection({
          projects: state.projects,
          tasks: state.tasks,
          sessions: state.sessions,
          treePending: false,
        }),
      client,
    );

    await waitFor(() => {
      expect(useWorkspaceSelectionStore.getState().selection.sessionId).toBe(
        "s1",
      );
    });
    expect(useUiStore.getState().expandedProjects.has("p1")).toBe(false);
    expect(useUiStore.getState().expandedTasks.has("t1")).toBe(false);
  });

  it("clears a stale session candidate without applying it", async () => {
    useWorkspaceSelectionStore.setState({
      selection: EMPTY_WORKSPACE_SELECTION,
      pendingRestore: {
        projectId: "p1",
        taskId: "t1",
        sessionId: "missing",
        workflowRunId: null,
        draftId: null,
      },
    });
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    state.sessions = [SESSION];
    const client = createMockClient(state);

    renderHookWithClient(
      () =>
        useRestoreWorkspaceSelection({
          projects: state.projects,
          tasks: state.tasks,
          sessions: state.sessions,
          treePending: false,
        }),
      client,
    );

    await waitFor(() => {
      expect(useWorkspaceSelectionStore.getState().pendingRestore).toBeNull();
    });
    expect(useWorkspaceSelectionStore.getState().selection).toEqual(
      EMPTY_WORKSPACE_SELECTION,
    );
  });

  it("applies the staged restore even if live selection was set before commit", async () => {
    // Direct setState simulates a stale in-memory selection; pendingRestore must
    // still win so startup chatter cannot keep a wrong leaf.
    useWorkspaceSelectionStore.setState({
      selection: {
        projectId: "p1",
        taskId: null,
        sessionId: null,
        workflowRunId: null,
        draftId: null,
      },
      pendingRestore: {
        projectId: "p1",
        taskId: "t1",
        sessionId: "s1",
        workflowRunId: null,
        draftId: null,
      },
    });
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    state.sessions = [SESSION];
    const client = createMockClient(state);

    renderHookWithClient(
      () =>
        useRestoreWorkspaceSelection({
          projects: state.projects,
          tasks: state.tasks,
          sessions: state.sessions,
          treePending: false,
        }),
      client,
    );

    await waitFor(() => {
      expect(useWorkspaceSelectionStore.getState().selection).toEqual({
        projectId: "p1",
        taskId: "t1",
        sessionId: "s1",
        workflowRunId: null,
        draftId: null,
      });
    });
    expect(useWorkspaceSelectionStore.getState().pendingRestore).toBeNull();
  });

  it("lets an explicit selectSession cancel a staged restore", () => {
    useWorkspaceSelectionStore.setState({
      selection: EMPTY_WORKSPACE_SELECTION,
      pendingRestore: {
        projectId: "p1",
        taskId: "t1",
        sessionId: "s1",
        workflowRunId: null,
        draftId: null,
      },
    });
    useWorkspaceSelectionStore.getState().selectSession("s-newest", "t1", "p1");
    expect(useWorkspaceSelectionStore.getState().selection.sessionId).toBe(
      "s-newest",
    );
    expect(useWorkspaceSelectionStore.getState().pendingRestore).toBeNull();
  });

  it("waits while the tree is still pending", async () => {
    useWorkspaceSelectionStore.setState({
      selection: EMPTY_WORKSPACE_SELECTION,
      pendingRestore: {
        projectId: "p1",
        taskId: "t1",
        sessionId: "s1",
        workflowRunId: null,
        draftId: null,
      },
    });
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    state.sessions = [SESSION];
    const client = createMockClient(state);

    renderHookWithClient(
      () =>
        useRestoreWorkspaceSelection({
          projects: state.projects,
          tasks: state.tasks,
          sessions: state.sessions,
          treePending: true,
        }),
      client,
    );

    await act(async () => {
      await Promise.resolve();
    });
    expect(useWorkspaceSelectionStore.getState().selection).toEqual(
      EMPTY_WORKSPACE_SELECTION,
    );
    expect(useWorkspaceSelectionStore.getState().pendingRestore).not.toBeNull();
  });

  it("does not miss-clear when the sessions list is empty before the tree is ready", async () => {
    // Regression: gating on `!isPending` alone let a failed/empty interim list
    // clear pendingRestore and persist the wipe — next launch then restored nothing.
    useWorkspaceSelectionStore.setState({
      selection: EMPTY_WORKSPACE_SELECTION,
      pendingRestore: {
        projectId: "p1",
        taskId: "t1",
        sessionId: "s1",
        workflowRunId: null,
        draftId: null,
      },
    });
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    state.sessions = [];
    const client = createMockClient(state);

    renderHookWithClient(
      () =>
        useRestoreWorkspaceSelection({
          projects: state.projects,
          tasks: state.tasks,
          sessions: state.sessions,
          treePending: true,
        }),
      client,
    );

    await act(async () => {
      await Promise.resolve();
    });
    expect(
      useWorkspaceSelectionStore.getState().pendingRestore?.sessionId,
    ).toBe("s1");
  });

  it("waits for draft rehydration before treating a draft candidate as missing", async () => {
    const finishListeners: Array<(state: unknown) => void> = [];
    const hasHydrated = vi
      .spyOn(useDraftSessionsStore.persist, "hasHydrated")
      .mockReturnValue(false);
    vi.spyOn(
      useDraftSessionsStore.persist,
      "onFinishHydration",
    ).mockImplementation((listener) => {
      finishListeners.push(listener as (state: unknown) => void);
      return () => {
        const index = finishListeners.indexOf(
          listener as (state: unknown) => void,
        );
        if (index >= 0) finishListeners.splice(index, 1);
      };
    });

    const draftId = useDraftSessionsStore
      .getState()
      .ensureEmptyDraft({ projectId: "p1", taskId: null });
    useDraftSessionsStore.getState().updateContent(draftId, { text: "parked" });

    useWorkspaceSelectionStore.setState({
      selection: EMPTY_WORKSPACE_SELECTION,
      pendingRestore: {
        projectId: "p1",
        taskId: null,
        sessionId: null,
        workflowRunId: null,
        draftId,
      },
    });

    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    state.sessions = [SESSION];
    const client = createMockClient(state);

    renderHookWithClient(
      () =>
        useRestoreWorkspaceSelection({
          projects: state.projects,
          tasks: state.tasks,
          sessions: state.sessions,
          treePending: false,
        }),
      client,
    );

    await act(async () => {
      await Promise.resolve();
    });
    expect(useWorkspaceSelectionStore.getState().pendingRestore).not.toBeNull();
    expect(useWorkspaceSelectionStore.getState().selection).toEqual(
      EMPTY_WORKSPACE_SELECTION,
    );

    hasHydrated.mockReturnValue(true);
    await act(async () => {
      for (const listener of finishListeners) listener({});
    });

    await waitFor(() => {
      expect(useWorkspaceSelectionStore.getState().selection).toEqual({
        projectId: "p1",
        taskId: null,
        sessionId: null,
        workflowRunId: null,
        draftId,
      });
    });
    expect(useWorkspaceSelectionStore.getState().pendingRestore).toBeNull();
  });

  it("clears a workflow-run candidate that lacks a project id instead of waiting forever", async () => {
    useWorkspaceSelectionStore.setState({
      selection: EMPTY_WORKSPACE_SELECTION,
      // Bypass sanitize: a corrupt in-memory candidate must not hang restore.
      pendingRestore: {
        projectId: null,
        taskId: null,
        sessionId: null,
        workflowRunId: "run-1",
        draftId: null,
      },
    });
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    state.sessions = [SESSION];
    const client = createMockClient(state);

    renderHookWithClient(
      () =>
        useRestoreWorkspaceSelection({
          projects: state.projects,
          tasks: state.tasks,
          sessions: state.sessions,
          treePending: false,
        }),
      client,
    );

    await waitFor(() => {
      expect(useWorkspaceSelectionStore.getState().pendingRestore).toBeNull();
    });
    expect(useWorkspaceSelectionStore.getState().selection).toEqual(
      EMPTY_WORKSPACE_SELECTION,
    );
  });

  it("applies a validated workflow run once its run list settles", async () => {
    useWorkspaceSelectionStore.setState({
      selection: EMPTY_WORKSPACE_SELECTION,
      pendingRestore: {
        projectId: "p1",
        taskId: null,
        sessionId: null,
        workflowRunId: "run-1",
        draftId: null,
      },
    });
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    state.sessions = [SESSION];
    state.workflowRuns = [
      {
        id: "run-1",
        projectId: "p1",
        workflowId: "wf-1",
        snapshotId: "snap-1",
        name: "Deploy",
        status: "succeeded",
        taskId: "t-run",
        createdAt: 0n,
        updatedAt: 0n,
      },
    ];
    const client = createMockClient(state);

    renderHookWithClient(
      () =>
        useRestoreWorkspaceSelection({
          projects: state.projects,
          tasks: state.tasks,
          sessions: state.sessions,
          treePending: false,
        }),
      client,
    );

    await waitFor(() => {
      expect(
        useWorkspaceSelectionStore.getState().selection.workflowRunId,
      ).toBe("run-1");
    });
    expect(useWorkspaceSelectionStore.getState().pendingRestore).toBeNull();
    expect(useUiStore.getState().expandedProjects.has("p1")).toBe(false);
  });

  it("waits while the workflow-run query has errored instead of discarding the candidate", async () => {
    // Offline restart: the run list query fails. A guardless implementation
    // would resolve against the empty error list, miss the run, and clear
    // pendingRestore — permanently losing the restore candidate.
    useWorkspaceSelectionStore.setState({
      selection: EMPTY_WORKSPACE_SELECTION,
      pendingRestore: {
        projectId: "p1",
        taskId: null,
        sessionId: null,
        workflowRunId: "run-1",
        draftId: null,
      },
    });
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    state.sessions = [SESSION];
    state.workflowRuns = [];
    const client = createMockClient(state);
    const listSpy = vi
      .spyOn(client.workflowRun, "list")
      .mockRejectedValue(new Error("offline"));

    const { queryClient } = renderHookWithClient(
      () =>
        useRestoreWorkspaceSelection({
          projects: state.projects,
          tasks: state.tasks,
          sessions: state.sessions,
          treePending: false,
        }),
      client,
    );

    await waitFor(() => {
      expect(
        queryClient.getQueryState(["workflowRun", "byProject", "p1"])?.status,
      ).toBe("error");
    });
    // Let the errored query re-render and re-run the effect so a guardless
    // implementation would have cleared the candidate by now.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(listSpy).toHaveBeenCalled();
    expect(useWorkspaceSelectionStore.getState().pendingRestore).not.toBeNull();
    expect(useWorkspaceSelectionStore.getState().selection).toEqual(
      EMPTY_WORKSPACE_SELECTION,
    );
  });

  it("preserves a user createFocus when applying a restored session", async () => {
    useWorkspaceSelectionStore.setState({
      selection: EMPTY_WORKSPACE_SELECTION,
      pendingRestore: {
        projectId: "p1",
        taskId: "t1",
        sessionId: "s1",
        workflowRunId: null,
        draftId: null,
      },
      createFocus: { projectId: "p2", taskId: null },
    });
    const state = createMockClientState();
    state.projects = [PROJECT, { id: "p2", name: "Other", rootPath: "/other" }];
    state.tasks = [TASK];
    state.sessions = [SESSION];
    const client = createMockClient(state);

    renderHookWithClient(
      () =>
        useRestoreWorkspaceSelection({
          projects: state.projects,
          tasks: state.tasks,
          sessions: state.sessions,
          treePending: false,
        }),
      client,
    );

    await waitFor(() => {
      expect(useWorkspaceSelectionStore.getState().selection.sessionId).toBe(
        "s1",
      );
    });
    expect(useWorkspaceSelectionStore.getState().createFocus).toEqual({
      projectId: "p2",
      taskId: null,
    });
  });
});

describe("useWarmSession restore gate", () => {
  it("does not warm while pendingRestore is set", async () => {
    useWorkspaceSelectionStore.setState({
      selection: {
        projectId: "p1",
        taskId: "t1",
        sessionId: null,
        workflowRunId: null,
        draftId: null,
      },
      pendingRestore: {
        projectId: "p1",
        taskId: "t1",
        sessionId: "s1",
        workflowRunId: null,
        draftId: null,
      },
    });
    const state = createMockClientState();
    state.projects = [PROJECT];
    state.tasks = [TASK];
    state.sessions = [];
    const client = createMockClient(state);
    const warm = vi.spyOn(client.session, "warm");

    renderHookWithClient(
      () =>
        useWarmSession(
          {
            projectId: "p1",
            taskId: "t1",
            sessionId: null,
          },
          "ora-space.opencode",
        ),
      client,
      undefined,
      createChatStore(client.session),
    );

    await act(async () => {
      await Promise.resolve();
    });
    expect(warm).not.toHaveBeenCalled();
  });
});
