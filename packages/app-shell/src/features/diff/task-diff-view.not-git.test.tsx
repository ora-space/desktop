import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import {
  RemoteContractError,
  type GetWorkspaceDiffRequest,
  type GetWorkspaceDiffResponse,
  type WorkspaceGit,
} from "@ora/contracts";
import { ContractsClientContext } from "../../contracts-client-context";
import { AppI18nProvider } from "../../i18n/i18n";
import "../../i18n/i18n-instance";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import { TaskDiffView, type TaskDiffFileRequest } from "./task-diff-view";

const DIFF: GetWorkspaceDiffResponse = {
  baseCommitId: null,
  headCommitId: "head",
  patch:
    "diff --git a/a.ts b/a.ts\nnew file mode 100644\nindex 0000000..1111111\n--- /dev/null\n+++ b/a.ts\n@@ -0,0 +1,1 @@\n+hello\n",
};

type WorkspaceList = Awaited<
  ReturnType<NonNullable<TestHandlers["listWorkspaces"]>>
>;

/** Lists workspace "A" with a present folder whose probe reports `git`. */
function listing(git: WorkspaceGit): WorkspaceList {
  return {
    workspaces: [
      {
        id: "A",
        projectId: "project",
        kind: "main" as const,
        lifecycle: "active" as const,
        directory: { kind: "present" as const, git },
      },
    ],
  };
}

/**
 * Mounts the Changes view for workspace "A". `list` is either the folder probe the workspace
 * list reports for it, or the list handler itself for failure and timing cases.
 */
function mountChanges(
  list: WorkspaceGit | (() => Promise<WorkspaceList>),
  fileRequest?: TaskDiffFileRequest,
) {
  const getWorkspaceDiff = vi.fn<
    (request: GetWorkspaceDiffRequest) => Promise<GetWorkspaceDiffResponse>
  >(async () => DIFF);
  const getWorkspaceStatus = vi.fn(async () => ({ entries: [] }));
  const listWorkspaces = vi.fn(
    typeof list === "function" ? list : async () => listing(list),
  );
  const onFileNotFound = vi.fn();
  const client = createTestClient({
    listWorkspaces,
    getWorkspaceDiff,
    getWorkspaceStatus,
  });
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  render(
    <QueryClientProvider client={queryClient}>
      <ContractsClientContext.Provider value={client}>
        <AppI18nProvider>
          <TaskDiffView
            workspaceId="A"
            hasBaseline={false}
            viewType="unified"
            fileTreeOpen
            fileRequest={fileRequest}
            toolbar={<button type="button">Files</button>}
            onFileTreeOpenChange={() => undefined}
            onFileNotFound={onFileNotFound}
          />
        </AppI18nProvider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>,
  );
  return {
    getWorkspaceDiff,
    getWorkspaceStatus,
    listWorkspaces,
    onFileNotFound,
  };
}

describe("TaskDiffView for a folder that is not a Git repository", () => {
  it("explains the folder instead of loading a diff, and keeps the panel toolbar", async () => {
    const { getWorkspaceDiff, getWorkspaceStatus } =
      mountChanges("notRepository");

    expect(await screen.findByText("此目录未使用 Git")).toBeInTheDocument();
    expect(screen.getByText(/文件会直接保存在原目录/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Files" })).toBeInTheDocument();
    expect(screen.queryByText("无法加载变更")).toBeNull();
    expect(getWorkspaceDiff).not.toHaveBeenCalled();
    expect(getWorkspaceStatus).not.toHaveBeenCalled();
  });

  it("re-probes the folder from the explanation", async () => {
    const { listWorkspaces } = mountChanges("notRepository");
    await screen.findByText("此目录未使用 Git");

    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: "重新检查" }));

    await waitFor(() => expect(listWorkspaces).toHaveBeenCalledTimes(2));
    expect(
      await screen.findByRole("button", { name: "重新检查" }),
    ).toBeEnabled();
  });

  it("hands a chat jump to the Files panel because there is no diff to show", async () => {
    const { onFileNotFound, getWorkspaceDiff } = mountChanges("notRepository", {
      path: "src/main.ts",
      requestId: 1,
      line: 4,
    });

    await waitFor(() =>
      expect(onFileNotFound).toHaveBeenCalledWith("src/main.ts", { line: 4 }),
    );
    expect(getWorkspaceDiff).not.toHaveBeenCalled();
  });

  it.each(["repository", "unknown"] as const)(
    "loads the diff as before when the probe reports %s",
    async (git) => {
      const { getWorkspaceDiff, getWorkspaceStatus } = mountChanges(git);

      expect(
        await screen.findByRole("button", { name: "暂存 a.ts" }),
      ).toBeInTheDocument();
      expect(screen.queryByText("此目录未使用 Git")).toBeNull();
      expect(getWorkspaceDiff.mock.calls[0]?.[0]).toEqual({
        workspaceId: "A",
        scope: "unstaged",
      });
      expect(getWorkspaceStatus).toHaveBeenCalled();
    },
  );

  it("loads the diff as before when the workspace list fails", async () => {
    const { getWorkspaceDiff } = mountChanges(async () => {
      throw new Error("list unavailable");
    });

    expect(
      await screen.findByRole("button", { name: "暂存 a.ts" }),
    ).toBeInTheDocument();
    expect(getWorkspaceDiff).toHaveBeenCalled();
  });

  it("loads the diff as before when the workspace is absent from the list", async () => {
    const { getWorkspaceDiff } = mountChanges(async () => ({ workspaces: [] }));

    expect(
      await screen.findByRole("button", { name: "暂存 a.ts" }),
    ).toBeInTheDocument();
    expect(getWorkspaceDiff).toHaveBeenCalled();
  });

  it("keeps a chat jump in Changes while the folder is still being checked", async () => {
    let releaseList!: (list: WorkspaceList) => void;
    const pendingList = new Promise<WorkspaceList>((resolve) => {
      releaseList = resolve;
    });
    const { onFileNotFound, getWorkspaceDiff, listWorkspaces } = mountChanges(
      () => pendingList,
      { path: "a.ts", requestId: 1 },
    );
    await waitFor(() => expect(listWorkspaces).toHaveBeenCalled());
    expect(screen.getByRole("status")).toHaveTextContent("正在加载变更…");
    expect(onFileNotFound).not.toHaveBeenCalled();
    expect(getWorkspaceDiff).not.toHaveBeenCalled();

    await act(async () => {
      releaseList(listing("repository"));
      await pendingList;
    });

    expect(
      await screen.findByRole("button", { name: "暂存 a.ts" }),
    ).toBeInTheDocument();
    expect(onFileNotFound).not.toHaveBeenCalled();
  });

  it("re-probes the folder when Git refuses it, then explains instead of a retry", async () => {
    const probes: WorkspaceGit[] = ["repository", "notRepository"];
    const { getWorkspaceDiff, listWorkspaces } = mountChanges(async () =>
      listing(probes.shift() ?? "notRepository"),
    );
    getWorkspaceDiff.mockRejectedValue(
      new RemoteContractError(
        {
          code: "workspace_not_git_repository",
          params: {},
          requestId: "550e8400-e29b-41d4-a716-446655440000",
        },
        null,
      ),
    );

    expect(await screen.findByText("此目录未使用 Git")).toBeInTheDocument();
    expect(listWorkspaces).toHaveBeenCalledTimes(2);
    expect(screen.queryByText("无法加载变更")).toBeNull();
  });
});
