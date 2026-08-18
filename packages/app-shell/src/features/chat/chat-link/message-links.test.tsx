import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PlatformProvider } from "@ora/platform";
import type { ChatToolCall, ChatTurn } from "@ora/chat";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import { ContractsClientContext } from "../../../contracts-client-context";
import { AppI18nProvider } from "../../../i18n/i18n";
import {
  createMockClient,
  createMockClientState,
} from "../../../test/mock-client";
import { createStubPlatform } from "../../../test/stub-platform";
import { TaskChangesNavigationProvider } from "../../diff/task-changes-navigation";
import { MessageList } from "../message-list";
import { MarkdownDocument, MarkdownMessage } from "../markdown-message";
import { ChatLinkContext } from "./context";
import type { SessionArtifactIndex } from "./artifact-index";

const index: SessionArtifactIndex = {
  edited: ["src/main.rs"],
  referenced: ["src/lib.rs"],
};

function renderLinkedMarkdown(content: string) {
  const openDiff = vi.fn();
  const openWorkspaceFile = vi.fn();
  render(
    <PlatformProvider adapter={createStubPlatform()}>
      <AppI18nProvider>
        <TaskChangesNavigationProvider
          onOpenDiff={openDiff}
          onOpenWorkspaceFile={openWorkspaceFile}
        >
          <ChatLinkContext.Provider value={{ index, taskId: "task-1" }}>
            <MarkdownMessage content={content} />
          </ChatLinkContext.Provider>
        </TaskChangesNavigationProvider>
      </AppI18nProvider>
    </PlatformProvider>,
  );
  return { openDiff, openWorkspaceFile };
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

describe("assistant markdown artifact links", () => {
  it("opens an edited inline path in Changes and a read path in Files", async () => {
    const user = userEvent.setup();
    const { openDiff } = renderLinkedMarkdown("See `src/main.rs`");
    await user.click(screen.getByRole("button", { name: /src\/main\.rs/ }));
    expect(openDiff).toHaveBeenCalledWith("src/main.rs", undefined);

    const read = renderLinkedMarkdown("See `src/lib.rs`");
    await user.click(screen.getByRole("button", { name: /src\/lib\.rs/ }));
    expect(read.openWorkspaceFile).toHaveBeenCalledWith(
      "src/lib.rs",
      undefined,
      undefined,
    );
  });

  it("passes :line through to Changes", async () => {
    const user = userEvent.setup();
    const { openDiff } = renderLinkedMarkdown("See `src/main.rs:12`");
    await user.click(screen.getByRole("button", { name: /src\/main\.rs/ }));
    expect(openDiff).toHaveBeenCalledWith("src/main.rs", 12);
  });

  it("keeps https links as target=_blank and blocks dangerous schemes", () => {
    renderLinkedMarkdown(
      "[docs](https://example.com) [xss](javascript:alert(1))",
    );
    expect(screen.getByRole("link", { name: "docs" })).toHaveAttribute(
      "target",
      "_blank",
    );
    // react-markdown strips javascript: hrefs; the leftover anchor must not navigate.
    expect(screen.getByText("xss").closest("a")).not.toHaveAttribute(
      "target",
      "_blank",
    );
  });

  it("treats a relative Markdown file href as a Files open", async () => {
    const user = userEvent.setup();
    const { openWorkspaceFile } = renderLinkedMarkdown(
      "[guide](docs/guide.md)",
    );
    await user.click(screen.getByRole("button", { name: /docs\/guide\.md/ }));
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "docs/guide.md",
      undefined,
      undefined,
    );
  });

  it("does not add chat links to MarkdownDocument even inside ChatLinkContext", () => {
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TaskChangesNavigationProvider
            onOpenDiff={vi.fn()}
            onOpenWorkspaceFile={vi.fn()}
          >
            <ChatLinkContext.Provider value={{ index, taskId: "task-1" }}>
              <MarkdownDocument content="See `src/main.rs`" />
            </ChatLinkContext.Provider>
          </TaskChangesNavigationProvider>
        </AppI18nProvider>
      </PlatformProvider>,
    );
    expect(screen.queryByRole("button", { name: /src\/main\.rs/ })).toBeNull();
    expect(screen.getByText("src/main.rs").tagName).toBe("CODE");
  });
});

function renderMessageList(
  turns: ChatTurn[],
  options: {
    openDiff?: (path: string, line?: number) => void;
    openWorkspaceFile?: (path: string, line?: number, column?: number) => void;
    workspaceRoot?: string;
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
  return render(
    <QueryClientProvider client={queryClient}>
      <ContractsClientContext.Provider value={mockClient}>
        <PlatformProvider adapter={createStubPlatform()}>
          <AppI18nProvider>
            <TaskChangesNavigationProvider
              onOpenDiff={options.openDiff ?? vi.fn()}
              onOpenWorkspaceFile={options.openWorkspaceFile ?? vi.fn()}
            >
              <MessageList
                taskId="task-1"
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
}

describe("session-wide chat links", () => {
  it("opens a later mention of an earlier edited file in Changes", async () => {
    const user = userEvent.setup();
    const openDiff = vi.fn();
    renderMessageList(
      [
        turn("turn-1", [editTool("src/main.rs")]),
        turn("turn-2", [], "Updated `src/main.rs`"),
      ],
      { openDiff },
    );

    await user.click(
      screen.getByRole("button", {
        name: /打开文件 src\/main\.rs|Open file src\/main\.rs/,
      }),
    );
    expect(openDiff).toHaveBeenCalledWith("src/main.rs", undefined);
  });

  it("opens a path that was only read in Files", async () => {
    const user = userEvent.setup();
    const openWorkspaceFile = vi.fn();
    renderMessageList(
      [turn("turn-1", [readTool("src/lib.rs")], "See `src/lib.rs`")],
      { openWorkspaceFile },
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
  });

  it("keeps a read-only Files link on an earlier turn even if a later turn edits the file", async () => {
    const user = userEvent.setup();
    const openDiff = vi.fn();
    const openWorkspaceFile = vi.fn();
    renderMessageList(
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

    // Turn 1 link was read-only at turn 1
    await user.click(buttons[0]!);
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "src/main.rs",
      undefined,
      undefined,
    );
    expect(openDiff).not.toHaveBeenCalled();

    // Turn 2 link includes the edit
    await user.click(buttons[1]!);
    expect(openDiff).toHaveBeenCalledWith("src/main.rs", undefined);
  });

  it("opens the full workspace-relative path when clicking a bare filename referencing a nested file", async () => {
    const user = userEvent.setup();
    const openWorkspaceFile = vi.fn();
    const fullPath =
      "packages/app-shell/src/features/chat/chat-link/chat-file-link.test.tsx";
    renderMessageList(
      [
        turn(
          "turn-1",
          [readTool(fullPath)],
          "Summary of `chat-file-link.test.tsx`",
        ),
      ],
      { openWorkspaceFile },
    );

    await user.click(
      screen.getByRole("button", {
        name: new RegExp(
          `打开文件 ${fullPath.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}|Open file ${fullPath.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`,
        ),
      }),
    );
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      fullPath,
      undefined,
      undefined,
    );
  });

  it("strips workspace cwd when clicking a bare filename referencing an absolute path", async () => {
    const user = userEvent.setup();
    const openWorkspaceFile = vi.fn();
    const workspaceRoot = "E:/claude_code_project/desktop";
    const relativePath =
      "packages/app-shell/src/features/chat/chat-link/chat-file-link.test.tsx";
    const absolutePath = `${workspaceRoot}/${relativePath}`;
    renderMessageList(
      [
        turn(
          "turn-1",
          [readTool(absolutePath)],
          "Summary of `chat-file-link.test.tsx`",
        ),
      ],
      { openWorkspaceFile, workspaceRoot },
    );

    const button = await screen.findByRole("button", {
      name: new RegExp(
        `打开文件 ${relativePath.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}|Open file ${relativePath.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`,
      ),
    });
    await user.click(button);
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      relativePath,
      undefined,
      undefined,
    );
  });
});
