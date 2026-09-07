import { describe, expect, it } from "vitest";
import { createTestClient } from "../contracts-transport";
import { createWorkspaceMemory, workspaceHandlers } from "./workspaces";

describe("workspace memory adapter through the production client", () => {
  it("owns project/task CRUD and their canonical/isolated workspace projections", async () => {
    const state = createWorkspaceMemory();
    const client = createTestClient(workspaceHandlers(state));
    const { project } = await client.project.create({
      name: "Project",
      mainWorkspacePath: "/repo",
    });
    const { task } = await client.task.create({
      projectId: project.id,
      title: "Task",
    });
    expect(await client.workspace.list({})).toEqual({
      workspaces: [
        {
          id: `workspace-${project.id}`,
          projectId: project.id,
          kind: "main",
          lifecycle: "active",
        },
        {
          id: task.workspaceId,
          projectId: project.id,
          kind: "isolated",
          lifecycle: "active",
        },
      ],
    });
    const updated = { ...task, title: "Updated" };
    expect(
      await client.task.update({ taskId: task.id, title: updated.title }),
    ).toEqual({ task: updated });
    expect(await client.task.get({ taskId: task.id })).toEqual({
      task: updated,
    });
    expect(await client.task.delete({ taskId: task.id })).toEqual({
      taskId: task.id,
      workspaceId: task.workspaceId,
    });
    expect(await client.task.list({})).toEqual({ tasks: [] });
    expect(await client.project.list({})).toEqual({ projects: [project] });
  });

  it("does not share fixture state or implicitly install a session adapter", async () => {
    const first = createTestClient(workspaceHandlers(createWorkspaceMemory()));
    const second = createTestClient(workspaceHandlers(createWorkspaceMemory()));
    await first.project.create({
      name: "Only first",
      mainWorkspacePath: "/repo",
    });
    expect(await second.project.list({})).toEqual({ projects: [] });
    await expect(first.session.list({})).rejects.toThrow(
      "Unconfigured test operation: listSessions",
    );
    await expect(
      first.appEvents.watch({})[Symbol.asyncIterator]().next(),
    ).rejects.toThrow("Unconfigured test operation: watchAppEvents");
  });
});
