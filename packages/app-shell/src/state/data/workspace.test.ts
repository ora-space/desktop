import type { Project, Session, Task, Workspace } from "@ora/contracts";
import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import {
  cacheDeletedProject,
  cacheDeletedTask,
  findSessionPlacement,
  workspaceKeys,
} from "./workspace";
import { sessionKeys } from "./sessions";
import { diffKeys, invalidateWorkspaceDiffs } from "./diff";

/** Uses both main and isolated workspaces, including a task missing from the workspace list. */
function aggregate() {
  const client = new QueryClient();
  const projects: Project[] = [
    { id: "p1", name: "Deleted" },
    { id: "p2", name: "Surviving" },
  ];
  const tasks: Task[] = [
    { id: "t1", projectId: "p1", workspaceId: "w1", title: "Task" },
    { id: "t2", projectId: "p2", workspaceId: "w2", title: "Survivor" },
  ];
  const workspaces: Workspace[] = [
    { id: "main", projectId: "p1", kind: "main", lifecycle: "active" },
    { id: "w2", projectId: "p2", kind: "isolated", lifecycle: "active" },
  ];
  const sessions: Session[] = ["main", "w1", "w2"].map((workspaceId) => ({
    id: `session-${workspaceId}`,
    workspaceId,
    agentRef: "agent",
    status: "running",
    title: null,
    historyState: { type: "writable" },
  }));
  client.setQueryData(workspaceKeys.projects, projects);
  client.setQueryData(workspaceKeys.tasks, tasks);
  client.setQueryData(workspaceKeys.workspaces, workspaces);
  client.setQueryData(sessionKeys.sessions, sessions);
  return { client, projects, tasks, workspaces, sessions };
}

describe("workspace aggregate cache coordination", () => {
  it("scrubs project main and task sessions together and returns exact UI cleanup ids", () => {
    const { client, projects, tasks, workspaces, sessions } = aggregate();
    const unrelated = diffKeys.workspaceDiff("w2", "unstaged");
    client.setQueryData(unrelated, []);
    expect(cacheDeletedProject(client, "p1")).toEqual({
      taskIds: new Set(["t1"]),
      sessionIds: ["session-main", "session-w1"],
      nextProjectId: "p2",
    });
    expect([
      client.getQueryData(workspaceKeys.projects),
      client.getQueryData(workspaceKeys.tasks),
      client.getQueryData(sessionKeys.sessions),
      client.getQueryData(workspaceKeys.workspaces),
    ]).toEqual([[projects[1]], [tasks[1]], [sessions[2]], workspaces]);
    expect(
      [
        workspaceKeys.projects,
        workspaceKeys.tasks,
        sessionKeys.sessions,
        workspaceKeys.workspaces,
        unrelated,
      ].map((key) => client.getQueryState(key)?.isInvalidated),
    ).toEqual([true, true, true, true, false]);
    client.clear();
  });

  it("deletes only the task workspace, leaving direct and other-project sessions", () => {
    const { client, tasks, sessions } = aggregate();
    expect(findSessionPlacement(client, sessions[1]!)).toEqual({
      task: tasks[0],
      projectId: "p1",
    });
    expect(findSessionPlacement(client, sessions[0]!)).toEqual({
      task: undefined,
      projectId: "p1",
    });
    expect(cacheDeletedTask(client, "t1", "w1")).toEqual(["session-w1"]);
    expect(client.getQueryData(sessionKeys.sessions)).toEqual([
      sessions[0],
      sessions[2],
    ]);
    expect(client.getQueryData(workspaceKeys.tasks)).toEqual([tasks[1]]);
    expect(client.getQueryState(workspaceKeys.projects)?.isInvalidated).toBe(
      false,
    );
    client.clear();
  });

  it("refreshes all diff scopes of one workspace only", async () => {
    const client = new QueryClient();
    const keys = [
      diffKeys.workspaceDiff("w1", "unstaged"),
      diffKeys.workspaceDiff("w1", "staged"),
      diffKeys.workspaceDiff("w1", "committed"),
      diffKeys.workspaceDiff("w2", "unstaged"),
    ];
    for (const key of keys) client.setQueryData(key, []);
    await invalidateWorkspaceDiffs(client, "w1");
    expect(keys.map((key) => client.getQueryState(key)?.isInvalidated)).toEqual(
      [true, true, true, false],
    );
    client.clear();
  });
});
