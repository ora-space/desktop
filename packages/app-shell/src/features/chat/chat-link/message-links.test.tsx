import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PlatformProvider } from "../../../platform";
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
import { ToolCallBlock } from "../tool-call-block";
import { MarkdownDocument, MarkdownMessage } from "../markdown-message";
import { ChatLinkContext } from "./context";
import {
  collectSessionArtifactIndex,
  type SessionArtifactIndex,
} from "./artifact-index";

const index: SessionArtifactIndex = {
  edited: ["src/main.rs"],
  referenced: ["src/lib.rs"],
};

/** Lets `resolveTaskCwd` settle so CI's stderr-as-failure gate stays quiet. */
async function flushDesktopCwd() {
  await act(async () => {
    await Promise.resolve();
  });
}

async function renderLinkedMarkdown(content: string) {
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
  await flushDesktopCwd();
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

function searchTool(
  text: string,
  locations: { path: string }[] = [],
): ChatToolCall {
  return {
    kind: "toolCall",
    id: "glob-md",
    title: "**/*.md",
    toolKind: "search",
    status: "completed",
    content: [
      {
        type: "content",
        content: { type: "text", text },
      },
    ],
    locations,
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
    const { openDiff } = await renderLinkedMarkdown("See `src/main.rs`");
    await user.click(screen.getByRole("button", { name: /src\/main\.rs/ }));
    expect(openDiff).toHaveBeenCalledWith("src/main.rs", undefined);

    const read = await renderLinkedMarkdown("See `src/lib.rs`");
    await user.click(screen.getByRole("button", { name: /src\/lib\.rs/ }));
    expect(read.openWorkspaceFile).toHaveBeenCalledWith(
      "src/lib.rs",
      undefined,
      undefined,
    );
  });

  it("passes :line through to Changes", async () => {
    const user = userEvent.setup();
    const { openDiff } = await renderLinkedMarkdown("See `src/main.rs:12`");
    await user.click(screen.getByRole("button", { name: /src\/main\.rs/ }));
    expect(openDiff).toHaveBeenCalledWith("src/main.rs", 12);
  });

  it("keeps https links as target=_blank and blocks dangerous schemes", async () => {
    await renderLinkedMarkdown(
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
    const { openWorkspaceFile } = await renderLinkedMarkdown(
      "[guide](docs/guide.md)",
    );
    const button = screen.getByRole("button", { name: /docs\/guide\.md/ });
    expect(button.className).toContain("decoration-dashed");
    expect(button).toHaveClass("text-sky-700");
    await user.click(button);
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "docs/guide.md",
      undefined,
      undefined,
    );
  });

  it("does not nest a second file link inside a Markdown file href", async () => {
    const { openDiff } = await renderLinkedMarkdown(
      "See [src/main.rs](src/main.rs) in the list:\n\n- [src/main.rs](src/main.rs)",
    );
    const buttons = screen.getAllByRole("button", { name: /src\/main\.rs/ });
    expect(buttons).toHaveLength(2);
    for (const button of buttons) {
      expect(button.querySelector("button")).toBeNull();
      expect(button.closest("a")).toBeNull();
    }
    expect(openDiff).not.toHaveBeenCalled();
  });

  it("keeps https links visually distinct from file citations", async () => {
    await renderLinkedMarkdown("[docs](https://example.com) and `src/lib.rs`");
    const web = screen.getByRole("link", { name: "docs" });
    expect(web.className).not.toContain("decoration-dashed");
    expect(web).toHaveAttribute("target", "_blank");
    expect(
      screen.getByRole("button", { name: /src\/lib\.rs/ }).className,
    ).toContain("decoration-dashed");
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

async function renderMessageList(
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
  await flushDesktopCwd();
  return view;
}

describe("session-wide chat links", () => {
  it("opens a later mention of an earlier edited file in Changes", async () => {
    const user = userEvent.setup();
    const openDiff = vi.fn();
    await renderMessageList(
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
    await renderMessageList(
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
    await renderMessageList(
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

  it("strips workspace cwd when clicking a bare filename referencing an absolute path", async () => {
    const user = userEvent.setup();
    const openWorkspaceFile = vi.fn();
    const workspaceRoot = "E:/claude_code_project/desktop";
    const relativePath =
      "packages/app-shell/src/features/chat/chat-link/chat-file-link.test.tsx";
    const absolutePath = `${workspaceRoot}/${relativePath}`;
    await renderMessageList(
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

  it("links markdown files listed after a project-root glob with no per-file locations", async () => {
    const user = userEvent.setup();
    const openWorkspaceFile = vi.fn();
    const workspaceRoot = "D:/project/desktop";
    await renderMessageList(
      [
        turn(
          "turn-1",
          [
            searchTool(
              "D:\\project\\desktop\\README.md\nD:\\project\\desktop\\docs\\guide.md",
              [{ path: "D:\\project\\desktop" }],
            ),
          ],
          "Markdown files:\n- `README.md`\n- `docs/guide.md`",
        ),
      ],
      { openWorkspaceFile, workspaceRoot },
    );

    await user.click(
      await screen.findByRole("button", {
        name: /打开文件 README\.md|Open file README\.md/,
      }),
    );
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "README.md",
      undefined,
      undefined,
    );

    await user.click(
      screen.getByRole("button", {
        name: /打开文件 docs\/guide\.md|Open file docs\/guide\.md/,
      }),
    );
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "docs/guide.md",
      undefined,
      undefined,
    );
  });

  it("links a bare README.md to the workspace root when nested README.md files were also listed", async () => {
    const user = userEvent.setup();
    const openWorkspaceFile = vi.fn();
    await renderMessageList(
      [
        turn(
          "turn-1",
          [
            readTool("README.md"),
            readTool("crates/engine/README.md"),
            readTool("docs/trading.md"),
          ],
          "Docs:\n- `README.md`\n- `docs/trading.md`\n- `crates/engine/README.md`",
        ),
      ],
      { openWorkspaceFile },
    );

    await user.click(
      screen.getByRole("button", {
        name: /打开文件 README\.md|Open file README\.md/,
      }),
    );
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "README.md",
      undefined,
      undefined,
    );
    expect(openWorkspaceFile).not.toHaveBeenCalledWith(
      "crates/engine/README.md",
      undefined,
      undefined,
    );
  });

  it("links path-only markdown list items that the glob already touched", async () => {
    const user = userEvent.setup();
    const openWorkspaceFile = vi.fn();
    await renderMessageList(
      [
        turn(
          "turn-1",
          [searchTool("README.md\ndocs/guide.md")],
          "Markdown files:\n- README.md\n- docs/guide.md",
        ),
      ],
      { openWorkspaceFile },
    );

    await user.click(
      await screen.findByRole("button", {
        name: /打开文件 docs\/guide\.md|Open file docs\/guide\.md/,
      }),
    );
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "docs/guide.md",
      undefined,
      undefined,
    );
  });

  it("links path lines inside a plaintext fenced file list", async () => {
    const user = userEvent.setup();
    const openWorkspaceFile = vi.fn();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TaskChangesNavigationProvider
            onOpenDiff={vi.fn()}
            onOpenWorkspaceFile={openWorkspaceFile}
          >
            <ChatLinkContext.Provider
              value={{
                index: {
                  edited: [],
                  referenced: ["README.md", "docs/guide.md"],
                },
                taskId: "task-1",
              }}
            >
              <MarkdownMessage content={"```\nREADME.md\ndocs/guide.md\n```"} />
            </ChatLinkContext.Provider>
          </TaskChangesNavigationProvider>
        </AppI18nProvider>
      </PlatformProvider>,
    );
    await flushDesktopCwd();

    expect(screen.getByTestId("chat-path-list")).toBeInTheDocument();
    await user.click(
      screen.getByRole("button", {
        name: /打开文件 docs\/guide\.md|Open file docs\/guide\.md/,
      }),
    );
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "docs/guide.md",
      undefined,
      undefined,
    );
  });

  it("keeps leading spaces in expanded tool dumps", async () => {
    const tool = searchTool("  README.md\n    docs/guide.md");
    const artifactIndex = collectSessionArtifactIndex([turn("turn-1", [tool])]);
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TaskChangesNavigationProvider
            onOpenDiff={vi.fn()}
            onOpenWorkspaceFile={vi.fn()}
          >
            <ChatLinkContext.Provider
              value={{
                index: artifactIndex,
                taskId: "task-1",
              }}
            >
              <ToolCallBlock tool={tool} expanded />
            </ChatLinkContext.Provider>
          </TaskChangesNavigationProvider>
        </AppI18nProvider>
      </PlatformProvider>,
    );
    await flushDesktopCwd();

    const output = screen.getByTestId("chat-tool-path-output");
    for (const line of output.querySelectorAll(":scope > div")) {
      expect(line).toHaveClass("whitespace-pre-wrap");
    }
    expect(output).toHaveTextContent("  README.md", {
      normalizeWhitespace: false,
    });
    expect(output).toHaveTextContent("    docs/guide.md", {
      normalizeWhitespace: false,
    });
  });

  it("does not remount fenced code when the session artifact index updates", async () => {
    function Harness({ referenced }: { referenced: string[] }) {
      return (
        <PlatformProvider adapter={createStubPlatform()}>
          <AppI18nProvider>
            <TaskChangesNavigationProvider
              onOpenDiff={vi.fn()}
              onOpenWorkspaceFile={vi.fn()}
            >
              <ChatLinkContext.Provider
                value={{
                  index: { edited: [], referenced },
                  taskId: "task-1",
                }}
              >
                <MarkdownMessage content={"```rust\nfn main() {}\n```"} />
              </ChatLinkContext.Provider>
            </TaskChangesNavigationProvider>
          </AppI18nProvider>
        </PlatformProvider>
      );
    }

    const { rerender } = render(<Harness referenced={["src/lib.rs"]} />);
    await flushDesktopCwd();
    fireEvent.click(
      screen.getByRole("button", { name: /收起代码|Collapse code/ }),
    );
    expect(
      screen.getByRole("button", { name: /展开代码|Expand code/ }),
    ).toHaveAttribute("aria-expanded", "false");

    rerender(<Harness referenced={["src/lib.rs", "README.md"]} />);
    expect(
      screen.getByRole("button", { name: /展开代码|Expand code/ }),
    ).toHaveAttribute("aria-expanded", "false");
  });

  it("turns glob tool dump lines into Files links after expanding the tool", async () => {
    const user = userEvent.setup();
    const openWorkspaceFile = vi.fn();
    const tool = searchTool("README.md\ndocs/guide.md", [
      { path: "D:/project/desktop" },
    ]);
    const artifactIndex = collectSessionArtifactIndex([turn("turn-1", [tool])]);
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TaskChangesNavigationProvider
            onOpenDiff={vi.fn()}
            onOpenWorkspaceFile={openWorkspaceFile}
          >
            <ChatLinkContext.Provider
              value={{
                index: artifactIndex,
                taskId: "task-1",
                cwd: "D:/project/desktop",
              }}
            >
              <ToolCallBlock tool={tool} expanded />
            </ChatLinkContext.Provider>
          </TaskChangesNavigationProvider>
        </AppI18nProvider>
      </PlatformProvider>,
    );
    await flushDesktopCwd();

    await user.click(
      await screen.findByRole("button", {
        name: /打开文件 docs\/guide\.md|Open file docs\/guide\.md/,
      }),
    );
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "docs/guide.md",
      undefined,
      undefined,
    );
  });

  it("does not turn shell commands in prose into file links", async () => {
    await renderMessageList(
      [
        turn(
          "turn-1",
          [searchTool("README.md")],
          "Run cargo test then open README.md",
        ),
      ],
      { workspaceRoot: "D:/project/desktop" },
    );

    expect(screen.queryByRole("button", { name: /cargo test/ })).toBeNull();
    expect(
      await screen.findByRole("button", {
        name: /打开文件 README\.md|Open file README\.md/,
      }),
    ).toBeInTheDocument();
  });

  it("links markdown table cells that name globbed files", async () => {
    const user = userEvent.setup();
    const openWorkspaceFile = vi.fn();
    await renderMessageList(
      [
        turn(
          "turn-1",
          [searchTool("README.md\ndocs/guide.md")],
          "| File | Role |\n| --- | --- |\n| docs/guide.md | guide |",
        ),
      ],
      { openWorkspaceFile },
    );

    await user.click(
      await screen.findByRole("button", {
        name: /打开文件 docs\/guide\.md|Open file docs\/guide\.md/,
      }),
    );
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "docs/guide.md",
      undefined,
      undefined,
    );
  });
});
