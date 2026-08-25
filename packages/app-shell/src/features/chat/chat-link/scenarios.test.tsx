import { createElement, type ReactNode } from "react";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RemoteContractError } from "@ora/contracts";
import type { ChatToolCall, ChatTurn } from "@ora/chat";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import { PlatformProvider, type PlatformAdapter } from "../../../platform";
import { AppI18nProvider } from "../../../i18n/i18n";
import { ContractsClientContext } from "../../../contracts-client-context";
import {
  createMockClient,
  createMockClientState,
} from "../../../test/mock-client";
import { createStubPlatform } from "../../../test/stub-platform";
import { TaskChangesNavigationProvider } from "../../diff/task-changes-navigation";
import { WorkspaceFilesView } from "../../files/workspace-files-view";
import { MessageList } from "../message-list";
import type { SessionArtifactIndex } from "./artifact-index";
import { ChatFileLink } from "./chat-file-link";
import { ChatLinkContext } from "./context";

const index: SessionArtifactIndex = {
  edited: ["src/main.rs"],
  referenced: ["src/lib.rs"],
};

function desktopPlatform(open = vi.fn()): PlatformAdapter {
  return {
    ...createStubPlatform(),
    locationActions: {
      resolveTaskCwd: async () => "C:/repo",
      resolveWorkspaceCwd: async () => "C:/repo",
      open,
    },
  };
}

/** Lets `resolveTaskCwd` settle so CI's stderr-as-failure gate stays quiet. */
async function flushDesktopCwd() {
  await act(async () => {
    await Promise.resolve();
  });
}

async function renderFileLink(
  raw: string,
  options?: {
    index?: SessionArtifactIndex;
    cwd?: string;
    platform?: PlatformAdapter;
    openDiff?: (path: string, line?: number) => void;
    openWorkspaceFile?: (path: string, line?: number, column?: number) => void;
  },
) {
  const openDiff = options?.openDiff ?? vi.fn();
  const openWorkspaceFile = options?.openWorkspaceFile ?? vi.fn();
  const view = render(
    <PlatformProvider adapter={options?.platform ?? createStubPlatform()}>
      <AppI18nProvider>
        <TaskChangesNavigationProvider
          onOpenDiff={openDiff}
          onOpenWorkspaceFile={openWorkspaceFile}
        >
          <ChatLinkContext.Provider
            value={{
              index: options?.index ?? index,
              taskId: "task-1",
              cwd: options?.cwd,
            }}
          >
            <ChatFileLink source="inline-code" raw={raw}>
              {raw}
            </ChatFileLink>
          </ChatLinkContext.Provider>
        </TaskChangesNavigationProvider>
      </AppI18nProvider>
    </PlatformProvider>,
  );
  await flushDesktopCwd();
  return { ...view, openDiff, openWorkspaceFile };
}

function editTool(path: string): ChatToolCall {
  return {
    kind: "toolCall",
    id: `edit-${path}`,
    title: `Edit ${path}`,
    toolKind: "edit",
    status: "completed",
    content: [{ type: "diff", path, oldText: "a", newText: "b" }],
    locations: [{ path }],
    createdAt: 10,
    updatedAt: 20,
  };
}

function readTool(path: string): ChatToolCall {
  return {
    kind: "toolCall",
    id: `read-${path}`,
    title: `Read ${path}`,
    toolKind: "read",
    status: "completed",
    content: [],
    locations: [{ path }],
    createdAt: 10,
    updatedAt: 20,
  };
}

function turn(
  id: string,
  items: ChatTurn["items"],
  markdown?: string,
): ChatTurn {
  return {
    id,
    userMessage: {
      kind: "message",
      id: `${id}-user`,
      role: "user",
      content: "prompt",
      createdAt: 1,
    },
    items: [
      ...items,
      ...(markdown === undefined
        ? []
        : [
            {
              kind: "message" as const,
              id: `${id}-assistant`,
              role: "assistant" as const,
              content: markdown,
              createdAt: 2,
            },
          ]),
    ],
    status: "completed",
    stopReason: null,
    error: null,
    createdAt: 1,
  };
}

async function renderMessageList(
  turns: ChatTurn[],
  options: {
    openDiff?: (path: string, line?: number) => void;
    openWorkspaceFile?: (path: string, line?: number, column?: number) => void;
    workspaceRoot?: string;
    taskId?: string;
    projectId?: string;
  } = {},
) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const mockClient = createMockClient(createMockClientState());
  if (options.workspaceRoot) {
    mockClient.task.getWorkspace = vi.fn(async () => ({
      workspace: { rootPath: options.workspaceRoot!, branchName: "main" },
    }));
  }
  if (options.projectId !== undefined && options.taskId === undefined) {
    mockClient.project.list = vi.fn(async () => ({
      projects: [
        {
          id: options.projectId!,
          name: "Ora",
          rootPath: options.workspaceRoot ?? "C:/repo",
        },
      ],
    }));
  }
  const view = render(
    <QueryClientProvider client={queryClient}>
      <ContractsClientContext.Provider value={mockClient}>
        <PlatformProvider adapter={createStubPlatform()}>
          <AppI18nProvider>
            <TaskChangesNavigationProvider
              onOpenDiff={options.openDiff ?? vi.fn()}
              onOpenWorkspaceFile={options.openWorkspaceFile ?? vi.fn()}
            >
              <MessageList
                taskId={
                  options.taskId ?? (options.projectId ? undefined : "task-1")
                }
                projectId={options.projectId}
                turns={turns}
                userName="Ada"
                isResponding={false}
              />
            </TaskChangesNavigationProvider>
          </AppI18nProvider>
        </PlatformProvider>
      </ContractsClientContext.Provider>
    </QueryClientProvider>,
  );
  await flushDesktopCwd();
  return view;
}

/** Renders Files after a chat click whose workspace read fails (deleted / missing). */
function renderMissingFilesPreview(path: string) {
  const client = createMockClient(createMockClientState());
  client.fileSystem.readWorkspaceFile = async () => {
    throw new RemoteContractError(
      {
        code: "file_system_path_not_found",
        params: {},
        requestId: "eb093a72-6961-4e9f-966a-3d5187958476",
      },
      null,
    );
  };
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: 0 } },
  });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(
      QueryClientProvider,
      { client: queryClient },
      createElement(
        ContractsClientContext.Provider,
        { value: client },
        createElement(AppI18nProvider, null, children),
      ),
    );
  return render(
    <WorkspaceFilesView
      projectId="project-1"
      taskId="task-1"
      hideHeader
      fileRequest={{ path, requestId: 1 }}
    />,
    { wrapper },
  );
}

describe("chat link usage scenarios", () => {
  describe("edited vs referenced", () => {
    it("opens an edited file in Changes and a read-only file in Files", async () => {
      const user = userEvent.setup();
      const edited = await renderFileLink("src/main.rs");
      await user.click(screen.getByRole("button", { name: /src\/main\.rs/ }));
      expect(edited.openDiff).toHaveBeenCalledWith("src/main.rs", undefined);
      expect(edited.openWorkspaceFile).not.toHaveBeenCalled();

      edited.unmount();
      const referenced = await renderFileLink("src/lib.rs");
      await user.click(screen.getByRole("button", { name: /src\/lib\.rs/ }));
      expect(referenced.openWorkspaceFile).toHaveBeenCalledWith(
        "src/lib.rs",
        undefined,
        undefined,
      );
      expect(referenced.openDiff).not.toHaveBeenCalled();
    });

    it("keeps a read-only mention on Files after a later turn edits the same path", async () => {
      const user = userEvent.setup();
      const openDiff = vi.fn();
      const openWorkspaceFile = vi.fn();
      await renderMessageList(
        [
          turn("turn-1", [readTool("src/main.rs")], "Summary of `src/main.rs`"),
          turn("turn-2", [editTool("src/main.rs")], "Updated `src/main.rs`"),
        ],
        { openDiff, openWorkspaceFile },
      );

      const buttons = screen.getAllByRole("button", {
        name: /打开文件 src\/main\.rs|Open file src\/main\.rs/,
      });
      expect(buttons).toHaveLength(2);

      await user.click(buttons[0]!);
      expect(openWorkspaceFile).toHaveBeenCalledWith(
        "src/main.rs",
        undefined,
        undefined,
      );
      expect(openDiff).not.toHaveBeenCalled();

      await user.click(buttons[1]!);
      expect(openDiff).toHaveBeenCalledWith("src/main.rs", undefined);
    });
    it("opens a read-only mention in Files for a project draft without a task", async () => {
      const user = userEvent.setup();
      const openDiff = vi.fn();
      const openWorkspaceFile = vi.fn();
      await renderMessageList(
        [turn("turn-1", [readTool("src/lib.rs")], "See `src/lib.rs`")],
        {
          openDiff,
          openWorkspaceFile,
          projectId: "project-1",
          workspaceRoot: "C:/repo",
        },
      );

      await user.click(
        screen.getByRole("button", {
          name: /打开文件 src\/lib\.rs|Open file src\/lib\.rs/,
        }),
      );
      expect(openWorkspaceFile).toHaveBeenCalledWith(
        "src/lib.rs",
        undefined,
        undefined,
      );
      expect(openDiff).not.toHaveBeenCalled();
    });
  });

  describe("manually deleted files", () => {
    it("still opens Changes for an edited path after the workspace file is gone", async () => {
      const user = userEvent.setup();
      const { openDiff, openWorkspaceFile } =
        await renderFileLink("src/main.rs");
      await user.click(screen.getByRole("button", { name: /src\/main\.rs/ }));
      expect(openDiff).toHaveBeenCalledWith("src/main.rs", undefined);
      expect(openWorkspaceFile).not.toHaveBeenCalled();
    });

    it("still opens Files for a referenced path after the workspace file is gone", async () => {
      const user = userEvent.setup();
      const { openDiff, openWorkspaceFile } =
        await renderFileLink("src/lib.rs");
      await user.click(screen.getByRole("button", { name: /src\/lib\.rs/ }));
      expect(openWorkspaceFile).toHaveBeenCalledWith(
        "src/lib.rs",
        undefined,
        undefined,
      );
      expect(openDiff).not.toHaveBeenCalled();
    });

    it("shows the localized missing-path copy when Files cannot read the clicked file", async () => {
      renderMissingFilesPreview("src/lib.rs");

      expect(
        await screen.findByText(
          /所选路径不存在|The selected path was not found/,
        ),
      ).toBeInTheDocument();
      expect(screen.queryByText(/Remote Ora request failed/)).toBeNull();
    });

    it("still hands the deleted file path to the system file manager", async () => {
      const user = userEvent.setup();
      const open = vi.fn();
      await renderFileLink("src/lib.rs", {
        platform: desktopPlatform(open),
        cwd: "C:/repo",
      });
      fireEvent.contextMenu(
        screen.getByRole("button", { name: /src\/lib\.rs/ }),
      );
      expect(
        screen.queryByRole("menuitem", {
          name: /在变更中查看|View in Changes/,
        }),
      ).toBeNull();
      await user.click(
        screen.getByRole("menuitem", { name: /文件管理器|Explorer/ }),
      );
      await waitFor(() => {
        expect(open).toHaveBeenCalledWith("explorer", "C:/repo/src/lib.rs");
      });
    });
  });

  describe("paths outside the task working directory", () => {
    it("does not link a relative mention of a file read outside the task worktree", async () => {
      const openWorkspaceFile = vi.fn();
      await renderMessageList(
        [
          turn(
            "turn-1",
            [readTool("D:/project/desktop/crates/acp/src/lib.rs")],
            "## `crates/acp/src/lib.rs` 总结",
          ),
        ],
        {
          openWorkspaceFile,
          workspaceRoot:
            "D:/project/desktop/.data/worktrees/f06fdb43-1297-4ba3-9143-a7a95ee85b0b",
        },
      );

      expect(
        await screen.findByText("crates/acp/src/lib.rs"),
      ).toBeInTheDocument();
      expect(
        screen.queryByRole("button", { name: /crates\/acp\/src\/lib\.rs/ }),
      ).toBeNull();
      expect(openWorkspaceFile).not.toHaveBeenCalled();
    });

    it("does not link an absolute path that stays outside the task cwd", async () => {
      await renderFileLink("C:/other/config.toml", {
        index: {
          edited: [],
          referenced: ["C:/other/config.toml"],
        },
        cwd: "C:/repo",
      });
      expect(screen.queryByRole("button")).toBeNull();
      expect(screen.getByText("C:/other/config.toml").tagName).toBe("CODE");
    });
  });
});
