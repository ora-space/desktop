import { describe, expect, it } from "vitest";

import { resolveActiveWorkspaceId } from "./resolve-active-workspace-id";

/** A task leg pointing at one isolated worktree. */
const TASKS = [{ id: "task-1", workspaceId: "ws-task-1" }];

/** A project's main checkout plus the task's isolated worktree. */
const WORKSPACES = [
  { id: "ws-main", projectId: "p1", kind: "main" as const },
  { id: "ws-task-1", projectId: "p1", kind: "isolated" as const },
];

describe("resolveActiveWorkspaceId", () => {
  it("returns null when a workflow run is selected, even when a task sits underneath it", () => {
    expect(
      resolveActiveWorkspaceId(
        { projectId: "p1", taskId: "task-1", workflowRunId: "run-1" },
        TASKS,
        WORKSPACES,
      ),
    ).toBe(null);
  });

  it("returns the selected task's workspace id", () => {
    expect(
      resolveActiveWorkspaceId(
        { projectId: "p1", taskId: "task-1", workflowRunId: null },
        TASKS,
        WORKSPACES,
      ),
    ).toBe("ws-task-1");
  });

  it("returns null while the selected task is not yet listed", () => {
    expect(
      resolveActiveWorkspaceId(
        { projectId: "p1", taskId: "missing", workflowRunId: null },
        TASKS,
        WORKSPACES,
      ),
    ).toBe(null);
  });

  it("returns the project's main workspace id when only a project is selected", () => {
    expect(
      resolveActiveWorkspaceId(
        { projectId: "p1", taskId: null, workflowRunId: null },
        TASKS,
        WORKSPACES,
      ),
    ).toBe("ws-main");
  });

  it("returns null while the selected project has no main workspace yet", () => {
    expect(
      resolveActiveWorkspaceId(
        { projectId: "p2", taskId: null, workflowRunId: null },
        TASKS,
        WORKSPACES,
      ),
    ).toBe(null);
  });

  it("returns null when nothing is selected", () => {
    expect(
      resolveActiveWorkspaceId(
        { projectId: null, taskId: null, workflowRunId: null },
        TASKS,
        WORKSPACES,
      ),
    ).toBe(null);
  });

  it("treats an undefined workflowRunId like null", () => {
    expect(
      resolveActiveWorkspaceId(
        { projectId: "p1", taskId: null },
        TASKS,
        WORKSPACES,
      ),
    ).toBe("ws-main");
  });
});
