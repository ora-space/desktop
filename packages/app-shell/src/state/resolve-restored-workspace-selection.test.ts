import { describe, expect, it } from "vitest";
import type {
  Project,
  Session,
  Task,
  WorkflowRunSummary,
} from "@ora/contracts";
import type { SessionDraft } from "./stores/draft-sessions-store";
import { resolveRestoredWorkspaceSelection } from "./resolve-restored-workspace-selection";
import type { WorkspaceSelection } from "./stores/sanitize-workspace-selection";

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
const WORKFLOW_RUN: WorkflowRunSummary = {
  id: "run-1",
  name: "Deploy",
  projectId: "p1",
  workflowId: "wf-1",
  status: "succeeded",
  startedAt: null,
  finishedAt: null,
  createdAt: 0n,
};

const empty: WorkspaceSelection = {
  projectId: null,
  taskId: null,
  sessionId: null,
  workflowRunId: null,
  draftId: null,
};

/** Builds a typed draft row with only the fields the resolver reads. */
function draft(overrides: Partial<SessionDraft> = {}): SessionDraft {
  return {
    id: "d1",
    projectId: "p1",
    taskId: null,
    text: "hello",
    images: [],
    retainedAttachments: false,
    pendingSessionId: null,
    returnTo: null,
    sendInFlight: false,
    updatedAt: 1,
    ...overrides,
  };
}

describe("resolveRestoredWorkspaceSelection", () => {
  it("returns miss for an empty candidate", () => {
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: empty,
        projects: [PROJECT],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [],
        workflowRuns: [],
      }),
    ).toEqual({ kind: "miss" });
  });

  it("resolves a session from authoritative task and project records", () => {
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "wrong-project",
          taskId: "wrong-task",
          sessionId: "s1",
          workflowRunId: null,
          draftId: null,
        },
        projects: [PROJECT],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [],
        workflowRuns: [],
      }),
    ).toEqual({
      kind: "ready",
      selection: {
        projectId: "p1",
        taskId: "t1",
        sessionId: "s1",
        workflowRunId: null,
        draftId: null,
      },
    });
  });

  it("misses when the session is absent from the list", () => {
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: "t1",
          sessionId: "gone",
          workflowRunId: null,
          draftId: null,
        },
        projects: [PROJECT],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [],
        workflowRuns: [],
      }),
    ).toEqual({ kind: "miss" });
  });

  it("misses when the session's project was deleted", () => {
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: "t1",
          sessionId: "s1",
          workflowRunId: null,
          draftId: null,
        },
        projects: [],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [],
        workflowRuns: [],
      }),
    ).toEqual({ kind: "miss" });
  });

  it("misses when the session exists but its task was deleted (orphan warm guard)", () => {
    // The task lookup is the guard PR #424 added to stop a stale session id from
    // warming a provider session whose owning task no longer exists.
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: "t1",
          sessionId: "s1",
          workflowRunId: null,
          draftId: null,
        },
        projects: [PROJECT],
        tasks: [],
        sessions: [SESSION],
        drafts: [],
        workflowRuns: [],
      }),
    ).toEqual({ kind: "miss" });
  });

  it("resolves a typed draft that still exists", () => {
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: null,
          sessionId: null,
          workflowRunId: null,
          draftId: "d1",
        },
        projects: [PROJECT],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [draft()],
        workflowRuns: [],
      }),
    ).toEqual({
      kind: "ready",
      selection: {
        projectId: "p1",
        taskId: null,
        sessionId: null,
        workflowRunId: null,
        draftId: "d1",
      },
    });
  });

  it("misses a draft that did not survive restart", () => {
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: null,
          sessionId: null,
          workflowRunId: null,
          draftId: "d1",
        },
        projects: [PROJECT],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [],
        workflowRuns: [],
      }),
    ).toEqual({ kind: "miss" });
  });

  it("resolves a draft whose task still exists and matches the project", () => {
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: "t1",
          sessionId: null,
          workflowRunId: null,
          draftId: "d1",
        },
        projects: [PROJECT],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [draft({ taskId: "t1" })],
        workflowRuns: [],
      }),
    ).toEqual({
      kind: "ready",
      selection: {
        projectId: "p1",
        taskId: "t1",
        sessionId: null,
        workflowRunId: null,
        draftId: "d1",
      },
    });
  });

  it("misses a draft whose task was deleted", () => {
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: "t1",
          sessionId: null,
          workflowRunId: null,
          draftId: "d1",
        },
        projects: [PROJECT],
        tasks: [],
        sessions: [SESSION],
        drafts: [draft({ taskId: "t1" })],
        workflowRuns: [],
      }),
    ).toEqual({ kind: "miss" });
  });

  it("misses a draft whose task moved to another project", () => {
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: "t1",
          sessionId: null,
          workflowRunId: null,
          draftId: "d1",
        },
        projects: [PROJECT],
        tasks: [{ ...TASK, projectId: "p2" }],
        sessions: [SESSION],
        drafts: [draft({ taskId: "t1" })],
        workflowRuns: [],
      }),
    ).toEqual({ kind: "miss" });
  });

  it("waits when workflow runs have not settled", () => {
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: null,
          sessionId: null,
          workflowRunId: "run-1",
          draftId: null,
        },
        projects: [PROJECT],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [],
        workflowRuns: null,
      }),
    ).toEqual({ kind: "waiting" });
  });

  it("resolves a workflow run that is still in the run list", () => {
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: null,
          sessionId: null,
          workflowRunId: "run-1",
          draftId: null,
        },
        projects: [PROJECT],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [],
        workflowRuns: [WORKFLOW_RUN],
      }),
    ).toEqual({
      kind: "ready",
      selection: {
        projectId: "p1",
        taskId: null,
        sessionId: null,
        workflowRunId: "run-1",
        draftId: null,
      },
    });
  });

  it("misses a workflow run absent from the run list", () => {
    // The project is still in the list; the miss comes from the empty run list.
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: null,
          sessionId: null,
          workflowRunId: "run-1",
          draftId: null,
        },
        projects: [PROJECT],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [],
        workflowRuns: [] as WorkflowRunSummary[],
      }),
    ).toEqual({ kind: "miss" });
  });

  it("misses a workflow run whose project was deleted", () => {
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: null,
          sessionId: null,
          workflowRunId: "run-1",
          draftId: null,
        },
        projects: [],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [],
        workflowRuns: [WORKFLOW_RUN],
      }),
    ).toEqual({ kind: "miss" });
  });

  it("misses a workflow run whose authoritative project differs from the candidate", () => {
    // Ownership is re-derived from the run record, so a corrupt persisted
    // projectId cannot retarget the restored run at another project.
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: null,
          sessionId: null,
          workflowRunId: "run-1",
          draftId: null,
        },
        projects: [PROJECT, { ...PROJECT, id: "p2", name: "Other" }],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [],
        workflowRuns: [{ ...WORKFLOW_RUN, projectId: "p2" }],
      }),
    ).toEqual({ kind: "miss" });
  });

  it("resolves project-only and task-only candidates", () => {
    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: null,
          sessionId: null,
          workflowRunId: null,
          draftId: null,
        },
        projects: [PROJECT],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [],
        workflowRuns: [],
      }),
    ).toEqual({
      kind: "ready",
      selection: {
        projectId: "p1",
        taskId: null,
        sessionId: null,
        workflowRunId: null,
        draftId: null,
      },
    });

    expect(
      resolveRestoredWorkspaceSelection({
        candidate: {
          projectId: "p1",
          taskId: "t1",
          sessionId: null,
          workflowRunId: null,
          draftId: null,
        },
        projects: [PROJECT],
        tasks: [TASK],
        sessions: [SESSION],
        drafts: [],
        workflowRuns: [],
      }),
    ).toEqual({
      kind: "ready",
      selection: {
        projectId: "p1",
        taskId: "t1",
        sessionId: null,
        workflowRunId: null,
        draftId: null,
      },
    });
  });
});
