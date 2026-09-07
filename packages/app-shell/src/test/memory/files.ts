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
  } satisfies TestHandlers;
}
