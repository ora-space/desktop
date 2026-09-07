import { createElement, type ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ContractsClientContext } from "../../contracts-client-context";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import "../../i18n/i18n-instance";
import {
  MAX_COMPOSER_FILE_ACTIONS,
  takeComposerFilePaths,
} from "./composer-actions";
import {
  FILE_MENTION_DEBOUNCE_MS,
  fileMentionMenuStatus,
  fileMentionStatusMessageKey,
  useComposerFileMentions,
} from "./use-composer-file-mentions";

function createWrapper(client: ReturnType<typeof createTestClient>) {
  const queryClient = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: 0 },
    },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return createElement(
      QueryClientProvider,
      { client: queryClient },
      createElement(
        ContractsClientContext.Provider,
        { value: client },
        children,
      ),
    );
  };
}

describe("takeComposerFilePaths", () => {
  it("dedupes and stops at the menu cap without scanning further", () => {
    const paths = Array.from(
      { length: MAX_COMPOSER_FILE_ACTIONS + 20 },
      (_, index) => `src/file-${index}.ts`,
    );
    paths.splice(3, 0, "src/file-0.ts");
    expect(takeComposerFilePaths(paths)).toEqual(
      Array.from(
        { length: MAX_COMPOSER_FILE_ACTIONS },
        (_, index) => `src/file-${index}.ts`,
      ),
    );
  });
});

describe("fileMention status helpers", () => {
  it("maps statuses onto menu chrome and i18n keys", () => {
    expect(fileMentionMenuStatus("loading")).toBe("loading");
    expect(fileMentionMenuStatus("need-project")).toBe("empty");
    expect(fileMentionStatusMessageKey("loading", "app")).toBe(
      "chat.actionMenu.filesSearching",
    );
    expect(fileMentionStatusMessageKey("empty", "")).toBe(
      "chat.actionMenu.filesTypeToSearch",
    );
    expect(fileMentionStatusMessageKey("empty", "zzz")).toBe(
      "chat.actionMenu.filesEmpty",
    );
    expect(fileMentionStatusMessageKey("need-project", "")).toBe(
      "chat.actionMenu.filesNeedProject",
    );
    expect(fileMentionStatusMessageKey("ready", "app")).toBeUndefined();
  });
});

describe("useComposerFileMentions", () => {
  it("keeps prior hits during debounce with selection locked, without a spinner", async () => {
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    clientHandlers.listWorkspaceDirectory = async () => ({
      path: "",
      entries: [
        {
          name: "README.md",
          path: "README.md",
          kind: "file",
          isSymbolicLink: false,
        },
      ],
    });
    clientHandlers.searchWorkspace = async () => ({
      results: [{ kind: "file", path: "src/app.ts" }],
      truncated: false,
    });

    const { result, rerender } = renderHook(
      ({ atQuery }) =>
        useComposerFileMentions({
          taskId: "task-1",
          projectId: "project-1",
          atQuery,
          enabled: true,
        }),
      {
        wrapper: createWrapper(client),
        initialProps: { atQuery: "" as string | null },
      },
    );

    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.entries).toEqual([
      { path: "README.md", kind: "file" },
    ]);
    expect(result.current.selectionLocked).toBe(false);

    rerender({ atQuery: "app" });
    expect(result.current.status).toBe("ready");
    expect(result.current.entries).toEqual([
      { path: "README.md", kind: "file" },
    ]);
    expect(result.current.selectionLocked).toBe(true);

    await act(async () => {
      await new Promise((resolve) =>
        setTimeout(resolve, FILE_MENTION_DEBOUNCE_MS + 30),
      );
    });
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.entries).toEqual([
      { path: "src/app.ts", kind: "file" },
    ]);
    expect(result.current.selectionLocked).toBe(false);
  });

  it("includes root directories ahead of files", async () => {
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    clientHandlers.listWorkspaceDirectory = async () => ({
      path: "",
      entries: [
        {
          name: "README.md",
          path: "README.md",
          kind: "file",
          isSymbolicLink: false,
        },
        {
          name: "src",
          path: "src",
          kind: "directory",
          isSymbolicLink: false,
        },
      ],
    });

    const { result } = renderHook(
      () =>
        useComposerFileMentions({
          taskId: "task-1",
          projectId: undefined,
          atQuery: "",
          enabled: true,
        }),
      { wrapper: createWrapper(client) },
    );

    await act(async () => {
      await new Promise((resolve) =>
        setTimeout(resolve, FILE_MENTION_DEBOUNCE_MS + 30),
      );
    });
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.entries).toEqual([
      { path: "src", kind: "directory" },
      { path: "README.md", kind: "file" },
    ]);
  });

  it("searches the project checkout when no task is selected", async () => {
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    const listProject = vi.fn(async () => ({
      path: "",
      entries: [
        {
          name: "src",
          path: "src",
          kind: "directory" as const,
          isSymbolicLink: false,
        },
      ],
    }));
    clientHandlers.listProjectDirectory = listProject;
    clientHandlers.listWorkspaceDirectory = vi.fn(async () => ({
      path: "",
      entries: [],
    }));

    const { result } = renderHook(
      () =>
        useComposerFileMentions({
          taskId: undefined,
          projectId: "project-1",
          atQuery: "",
          enabled: true,
        }),
      { wrapper: createWrapper(client) },
    );

    await act(async () => {
      await new Promise((resolve) =>
        setTimeout(resolve, FILE_MENTION_DEBOUNCE_MS + 30),
      );
    });
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(listProject).toHaveBeenCalled();
    expect(clientHandlers.listWorkspaceDirectory).not.toHaveBeenCalled();
    expect(result.current.entries).toEqual([
      { path: "src", kind: "directory" },
    ]);
  });

  it("prefers the task worktree over the project checkout", async () => {
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    clientHandlers.listWorkspaceDirectory = async () => ({
      path: "",
      entries: [
        {
          name: "task-only.ts",
          path: "task-only.ts",
          kind: "file",
          isSymbolicLink: false,
        },
      ],
    });
    clientHandlers.listProjectDirectory = vi.fn(async () => ({
      path: "",
      entries: [
        {
          name: "project-only.ts",
          path: "project-only.ts",
          kind: "file" as const,
          isSymbolicLink: false,
        },
      ],
    }));

    const { result } = renderHook(
      () =>
        useComposerFileMentions({
          taskId: "task-1",
          projectId: "project-1",
          atQuery: "",
          enabled: true,
        }),
      { wrapper: createWrapper(client) },
    );

    await act(async () => {
      await new Promise((resolve) =>
        setTimeout(resolve, FILE_MENTION_DEBOUNCE_MS + 30),
      );
    });
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(clientHandlers.listProjectDirectory).not.toHaveBeenCalled();
    expect(result.current.entries).toEqual([
      { path: "task-only.ts", kind: "file" },
    ]);
  });

  it("caps search hits at the menu limit even when the API returns more", async () => {
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    clientHandlers.searchWorkspace = async () => ({
      results: Array.from({ length: 40 }, (_, index) => ({
        kind: "file" as const,
        path: `pkg/file-${index}.ts`,
      })),
      truncated: true,
    });

    const { result } = renderHook(
      () =>
        useComposerFileMentions({
          taskId: "task-1",
          projectId: undefined,
          atQuery: "file",
          enabled: true,
        }),
      { wrapper: createWrapper(client) },
    );

    await act(async () => {
      await new Promise((resolve) =>
        setTimeout(resolve, FILE_MENTION_DEBOUNCE_MS + 30),
      );
    });
    await waitFor(() => expect(result.current.status).toBe("ready"));
    expect(result.current.entries).toHaveLength(MAX_COMPOSER_FILE_ACTIONS);
    expect(result.current.truncated).toBe(true);
  });

  it("reports error instead of an empty hit list when search fails", async () => {
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    clientHandlers.searchWorkspace = async () => {
      throw new Error("search failed");
    };

    const { result } = renderHook(
      () =>
        useComposerFileMentions({
          taskId: "task-1",
          projectId: undefined,
          atQuery: "app",
          enabled: true,
        }),
      { wrapper: createWrapper(client) },
    );

    await act(async () => {
      await new Promise((resolve) =>
        setTimeout(resolve, FILE_MENTION_DEBOUNCE_MS + 30),
      );
    });
    await waitFor(() => expect(result.current.status).toBe("error"));
    expect(result.current.entries).toEqual([]);
  });

  it("asks for a project when none is selected", () => {
    const clientHandlers: TestHandlers = {};
    const client = createTestClient(clientHandlers);
    const { result } = renderHook(
      () =>
        useComposerFileMentions({
          taskId: undefined,
          projectId: undefined,
          atQuery: "",
          enabled: true,
        }),
      { wrapper: createWrapper(client) },
    );
    expect(result.current.status).toBe("need-project");
    expect(result.current.entries).toEqual([]);
  });
});
