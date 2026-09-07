import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import { invalidateScopedFileQueries, resolveFilesScope } from "./files";
import { fileKeys } from "./files";

describe("resolveFilesScope", () => {
  it("prefers the task worktree when both ids are present", () => {
    expect(resolveFilesScope("project-1", "task-1")).toEqual({
      kind: "task",
      taskId: "task-1",
    });
  });

  it("falls back to the project checkout without a task", () => {
    expect(resolveFilesScope("project-1", undefined)).toEqual({
      kind: "project",
      projectId: "project-1",
    });
  });
});

describe("file query invalidation", () => {
  it("invalidates both rename paths and searches, but no unrelated scope or directory", async () => {
    const client = new QueryClient();
    const keys = {
      oldFile: fileKeys.workspaceFile("task-1", "old/a.rs"),
      newFile: fileKeys.workspaceFile("task-1", "new/b.rs"),
      oldDirectory: fileKeys.workspaceDirectory("task-1", "old"),
      newDirectory: fileKeys.workspaceDirectory("task-1", "new"),
      search: fileKeys.workspaceSearch("task-1", "content", "hello"),
      unrelatedDirectory: fileKeys.workspaceDirectory("task-1", "other"),
      unrelatedFile: fileKeys.workspaceFile("task-1", "other/c.rs"),
      anotherTask: fileKeys.workspaceFile("task-2", "new/b.rs"),
      projectCheckout: fileKeys.projectFile("task-1", "new/b.rs"),
    };
    for (const key of Object.values(keys)) client.setQueryData(key, []);
    await invalidateScopedFileQueries(
      client,
      resolveFilesScope("project-1", "task-1"),
      [
        { kind: "renamed", from: "old/a.rs", path: "new/b.rs" },
        { kind: "modified", path: "new/b.rs" },
      ],
    );
    expect(
      Object.fromEntries(
        Object.entries(keys).map(([name, key]) => [
          name,
          client.getQueryState(key)?.isInvalidated,
        ]),
      ),
    ).toEqual({
      oldFile: true,
      newFile: true,
      oldDirectory: true,
      newDirectory: true,
      search: true,
      unrelatedDirectory: false,
      unrelatedFile: false,
      anotherTask: false,
      projectCheckout: false,
    });
    client.clear();
  });

  it("rescans every project projection without invalidating another checkout or task", async () => {
    const client = new QueryClient();
    const scoped = [
      fileKeys.projectDirectory("project-1", "src"),
      fileKeys.projectFile("project-1", "src/main.rs"),
      fileKeys.projectSearch("project-1", "content", "hello"),
    ];
    const other = [
      fileKeys.projectFile("project-2", "src/main.rs"),
      fileKeys.workspaceFile("project-1", "src/main.rs"),
    ];
    for (const key of [...scoped, ...other]) client.setQueryData(key, []);
    await invalidateScopedFileQueries(
      client,
      resolveFilesScope("project-1", undefined),
      [{ kind: "rescanRequired" }],
    );
    expect(
      [...scoped, ...other].map(
        (key) => client.getQueryState(key)?.isInvalidated,
      ),
    ).toEqual([true, true, true, false, false]);
    client.clear();
  });
});
