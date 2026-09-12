import type { TestHandlers } from "../contracts-transport";

/** Registers only the files operations explicitly requested by a fixture. */
export function emptyFilesHandlers() {
  return {
    listWorkspaceDirectory: async () => ({ path: "", entries: [] }),
    listProjectDirectory: async () => ({ path: "", entries: [] }),
    readProjectFile: async (request) => ({
      path: request.path,
      content: "",
      version: "test",
      sizeBytes: 0,
    }),
    readWorkspaceFile: async (request) => ({
      path: request.path,
      content: "",
      version: "test",
      sizeBytes: 0,
    }),
    searchWorkspace: async () => ({ results: [], truncated: false }),
    searchProject: async () => ({ results: [], truncated: false }),
    watchWorkspace: () =>
      (async function* () {
        yield* [];
      })(),
    watchProject: () =>
      (async function* () {
        yield* [];
      })(),
    createWorkspaceEntry: async ({ path, kind }) => ({
      name: path.split("/").pop() ?? path,
      path,
      kind,
      isSymbolicLink: false,
    }),
    createProjectEntry: async ({ path, kind }) => ({
      name: path.split("/").pop() ?? path,
      path,
      kind,
      isSymbolicLink: false,
    }),
    copyWorkspaceEntry: async ({ path }) => ({
      name: path.split("/").pop() ?? path,
      path,
      kind: "file",
      isSymbolicLink: false,
    }),
    copyProjectEntry: async ({ path }) => ({
      name: path.split("/").pop() ?? path,
      path,
      kind: "file",
      isSymbolicLink: false,
    }),
    moveWorkspaceEntry: async ({ path }) => ({
      name: path.split("/").pop() ?? path,
      path,
      kind: "file",
      isSymbolicLink: false,
    }),
    moveProjectEntry: async ({ path }) => ({
      name: path.split("/").pop() ?? path,
      path,
      kind: "file",
      isSymbolicLink: false,
    }),
    deleteWorkspaceEntry: async () => ({}),
    deleteProjectEntry: async () => ({}),
  } satisfies TestHandlers;
}
