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

async function renderLinkedMarkdown(
  content: string,
  options: { index?: SessionArtifactIndex; cwd?: string } = {},
) {
  const openDiff = vi.fn();
  const openWorkspaceFile = vi.fn();
  const openExternalUrl = vi.fn().mockResolvedValue(undefined);
  render(
    <PlatformProvider adapter={{ ...createStubPlatform(), openExternalUrl }}>
      <AppI18nProvider>
        <TaskChangesNavigationProvider
          onOpenDiff={openDiff}
          onOpenWorkspaceFile={openWorkspaceFile}
        >
          <ChatLinkContext.Provider
            value={{
              index: options.index ?? index,
              taskId: "task-1",
              cwd: options.cwd,
            }}
          >
            <MarkdownMessage content={content} />
          </ChatLinkContext.Provider>
        </TaskChangesNavigationProvider>
      </AppI18nProvider>
    </PlatformProvider>,
  );
  await flushDesktopCwd();
  return { openDiff, openWorkspaceFile, openExternalUrl };
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

function directoryListingTool(text: string): ChatToolCall {
  return {
    ...searchTool(text),
    id: "list-directory",
    title: "Get-ChildItem -Name",
    toolKind: "execute",
    rawInput: { command: "Get-ChildItem -Name" },
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

  it("opens https links through the platform and blocks dangerous schemes", async () => {
    const user = userEvent.setup();
    const { openExternalUrl } = await renderLinkedMarkdown(
      "[docs](https://example.com) [xss](javascript:alert(1))",
    );
    const docsLink = screen.getByRole("link", { name: "docs" });
    expect(docsLink).toHaveAttribute("target", "_blank");

    // Desktop's main window has no `on_new_window` hook (see surface/hooks.rs),
    // so a bare target="_blank" anchor never leaves the webview there even
    // though it looks clickable under jsdom. The click must go through the
    // same openExternalUrl command the prompt box uses instead.
    await user.click(docsLink);
    expect(openExternalUrl).toHaveBeenCalledWith("https://example.com");

    // react-markdown strips javascript: hrefs; the leftover anchor must not navigate.
    // Assistant anchors bypass react-markdown's URL filter, so the href is not
    // stripped upstream of this module; the classifier marks the scheme inert
    // and it renders as plain text rather than a dead anchor.
    expect(screen.getByText("xss").closest("a")).toBeNull();
    expect(openExternalUrl).not.toHaveBeenCalledWith(
      expect.stringContaining("javascript:"),
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

  it.each([
    "file:///C:/repo/docs/foo%20bar.md:12:3",
    "C:/repo/docs/foo%20bar.md#L12C3",
    "C:%5Crepo%5Cdocs%5Cfoo%20bar.md#L12C3",
    "/C:/repo/docs/foo%20bar.md?line=12&column=3",
  ])("preserves and opens Windows Markdown href %s", async (href) => {
    const user = userEvent.setup();
    const { openWorkspaceFile } = await renderLinkedMarkdown(
      `[guide](<${href}>)`,
      {
        cwd: "C:/repo",
        index: { edited: [], referenced: ["docs/foo bar.md"] },
      },
    );

    await user.click(screen.getByRole("button", { name: /docs\/foo bar\.md/ }));
    expect(openWorkspaceFile).toHaveBeenCalledWith("docs/foo bar.md", 12, 3);
  });

  it("keeps dangerous assistant hrefs out of the DOM", async () => {
    // React drops an empty `src` but warns about it first, and that warning is
    // deduplicated per worker: whichever test file renders one first fails the
    // clean-stderr gate. Asserting the missing attribute alone cannot see the
    // difference, so watch the warning itself.
    const consoleError = vi
      .spyOn(console, "error")
      .mockImplementation(() => undefined);
    await renderLinkedMarkdown(
      "[js](javascript:alert(1)) [data](data:text/html,boom) ![local](file:///C:/secret.png)",
    );
    expect(screen.getByText(/js\s+data/)).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "js" })).toBeNull();
    expect(screen.queryByRole("link", { name: "data" })).toBeNull();
    expect(screen.getByAltText("local")).not.toHaveAttribute("src");
    expect(consoleError).not.toHaveBeenCalled();
    consoleError.mockRestore();
  });

  it("links prose paths with CJK punctuation and natural line locations", async () => {
    const user = userEvent.setup();
    const { openWorkspaceFile } = await renderLinkedMarkdown(
      "查看 src/lib.rs (line 12, column 3)。",
    );
    await user.click(screen.getByRole("button", { name: /src\/lib\.rs/ }));
    expect(openWorkspaceFile).toHaveBeenCalledWith("src/lib.rs", 12, 3);
  });

  it("does not nest a second file link inside a Markdown file href", async () => {
    await renderLinkedMarkdown(
      "See [src/main.rs](src/main.rs) in the list:\n\n- [src/main.rs](src/main.rs)",
    );
    const buttons = screen.getAllByRole("button", { name: /src\/main\.rs/ });
    expect(buttons).toHaveLength(2);
    for (const button of buttons) {
      expect(button.querySelector("button")).toBeNull();
      expect(button.closest("a")).toBeNull();
    }
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

  it("keeps local hrefs filtered when assistant Markdown has no navigation context", () => {
    render(<MarkdownMessage content="[local](file:///C:/secret.txt)" />);
    // No context means react-markdown's default filter runs, so the local path
    // never reaches the DOM and the anchor is left with nothing to navigate to.
    expect(screen.getByText("local").closest("a")).toHaveAttribute("href", "");
  });
});

async function renderMessageList(
  turns: ChatTurn[],
  options: {
    openDiff?: (path: string, line?: number) => void;
    openWorkspaceFile?: (path: string, line?: number, column?: number) => void;
    openWorkspaceDirectory?: (path: string) => void;
    openWorkspaceArtifact?: (
      path: string,
      line?: number,
      column?: number,
    ) => void;
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
              onOpenWorkspaceDirectory={options.openWorkspaceDirectory}
              onOpenWorkspaceArtifact={options.openWorkspaceArtifact}
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
    const openDiff = vi.fn();
    const openWorkspaceArtifact = vi.fn();
    await renderMessageList(
      [turn("turn-1", [readTool("src/lib.rs")], "See `src/lib.rs`")],
      { openWorkspaceFile, openDiff, openWorkspaceArtifact },
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
    expect(openWorkspaceArtifact).not.toHaveBeenCalled();
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

  it("opens ripgrep output at its reported line and column", async () => {
    const user = userEvent.setup();
    const openWorkspaceFile = vi.fn();
    const tool = searchTool("src/lib.rs:12:3:export function run() {}", []);
    const artifactIndex = collectSessionArtifactIndex([turn("turn-1", [tool])]);
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TaskChangesNavigationProvider
            onOpenDiff={vi.fn()}
            onOpenWorkspaceFile={openWorkspaceFile}
          >
            <ChatLinkContext.Provider
              value={{ index: artifactIndex, taskId: "task-1" }}
            >
              <ToolCallBlock tool={tool} expanded />
            </ChatLinkContext.Provider>
          </TaskChangesNavigationProvider>
        </AppI18nProvider>
      </PlatformProvider>,
    );
    await flushDesktopCwd();

    await user.click(
      screen.getByRole("button", {
        name: /打开文件 src\/lib\.rs|Open file src\/lib\.rs/,
      }),
    );
    expect(openWorkspaceFile).toHaveBeenCalledWith("src/lib.rs", 12, 3);
  });

  it("opens absolute and slash-terminated tool directories in the Files tree", async () => {
    const user = userEvent.setup();
    const openWorkspaceDirectory = vi.fn();
    const openWorkspaceArtifact = vi.fn();
    const tool = directoryListingTool("C:\\repo\\cli\ndocs/");
    const artifactIndex = collectSessionArtifactIndex([turn("turn-1", [tool])]);
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TaskChangesNavigationProvider
            onOpenDiff={vi.fn()}
            onOpenWorkspaceFile={vi.fn()}
            onOpenWorkspaceDirectory={openWorkspaceDirectory}
            onOpenWorkspaceArtifact={openWorkspaceArtifact}
          >
            <ChatLinkContext.Provider
              value={{
                index: artifactIndex,
                taskId: "task-1",
                cwd: "C:/repo",
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
      screen.getByRole("button", {
        name: /打开路径 cli|Open path cli/,
      }),
    );
    await user.click(
      screen.getByRole("button", {
        name: /打开路径 docs|Open path docs/,
      }),
    );
    expect(openWorkspaceArtifact.mock.calls).toEqual([
      ["cli", undefined, undefined],
      ["docs"],
    ]);
    expect(openWorkspaceDirectory).not.toHaveBeenCalled();
  });

  it("links bare directories and extensionless files from the real name-list output", async () => {
    const user = userEvent.setup();
    const openWorkspaceDirectory = vi.fn();
    const openWorkspaceFile = vi.fn();
    const openWorkspaceArtifact = vi.fn();
    const output =
      ".git\n.github\ncli\ndocs\nhub\nnode_modules\nscripts\nshared\nweb\nwebsite\n.gitignore\nAGENTS.md\nbun.lock\nCONTRIBUTING.md\nLICENSE\npackage.json";
    await renderMessageList(
      [
        turn(
          "turn-1",
          [directoryListingTool(output)],
          "**目录**\n- `cli`\n- `docs`\n\n**文件**\n- `LICENSE`\n- `AGENTS.md`",
        ),
      ],
      {
        workspaceRoot: "C:/repo",
        openWorkspaceDirectory,
        openWorkspaceFile,
        openWorkspaceArtifact,
      },
    );

    await user.click(
      screen.getByRole("button", { name: /打开路径 cli|Open path cli/ }),
    );
    await user.click(
      screen.getByRole("button", {
        name: /打开路径 LICENSE|Open path LICENSE/,
      }),
    );
    expect(openWorkspaceDirectory).not.toHaveBeenCalled();
    expect(openWorkspaceFile).not.toHaveBeenCalled();
    expect(openWorkspaceArtifact.mock.calls).toEqual([
      ["cli", undefined, undefined],
      ["LICENSE", undefined, undefined],
    ]);
  });

  it("links a PowerShell listing delivered with locations and rawOutput", async () => {
    const user = userEvent.setup();
    const openWorkspaceFile = vi.fn();
    const openWorkspaceArtifact = vi.fn();
    const esc = String.fromCharCode(27);
    const output = [
      "",
      `${esc}[32;1mName           ${esc}[0m${esc}[32;1m PSIsContainer${esc}[0m`,
      `${esc}[32;1m----           ${esc}[0m ${esc}[32;1m-------------${esc}[0m`,
      "docs                     True",
      "main.py                 False",
      "",
    ].join(String.fromCharCode(13, 10));
    // A real ACP execute call carries the same text twice (visible content and
    // `rawOutput`) plus the cwd location, so each entry reaches the index both
    // qualified with its listing root and bare.
    const listing: ChatToolCall = {
      kind: "toolCall",
      id: "ps-list",
      title: "Get-ChildItem -Force | Select-Object Name, PSIsContainer",
      toolKind: "execute",
      status: "completed",
      content: [{ type: "content", content: { type: "text", text: output } }],
      locations: [{ path: "C:/repo" }],
      rawInput: {
        command: "Get-ChildItem -Force | Select-Object Name, PSIsContainer",
        cwd: "C:/repo",
      },
      rawOutput: { output },
      createdAt: 10,
      updatedAt: 20,
    };
    await renderMessageList(
      [
        turn(
          "turn-1",
          [listing],
          ["**目录**：`docs`", "", "**文件**：`main.py`"].join(
            String.fromCharCode(10),
          ),
        ),
      ],
      { workspaceRoot: "C:/repo", openWorkspaceFile, openWorkspaceArtifact },
    );

    await user.click(
      screen.getByRole("button", { name: /打开路径 docs|Open path docs/ }),
    );
    await user.click(
      screen.getByRole("button", {
        name: /打开文件 main.py|Open file main.py/,
      }),
    );
    // The directory branch resolves its kind from the parent listing, so it
    // hands over the path alone; files carry their line/column slots.
    expect(openWorkspaceArtifact.mock.calls).toEqual([["docs"]]);
    expect(openWorkspaceFile.mock.calls).toEqual([
      ["main.py", undefined, undefined],
    ]);
  });

  it("links indexed bare names in plain lists and tree fences", async () => {
    const user = userEvent.setup();
    const openWorkspaceDirectory = vi.fn();
    const openWorkspaceArtifact = vi.fn();
    const output = "cli\ndocs\nLICENSE\nAGENTS.md";
    await renderMessageList(
      [
        turn(
          "turn-1",
          [directoryListingTool(output)],
          "目录：\n- cli\n\n```text\nC:\\repo\n├── docs\n└── LICENSE\n```",
        ),
      ],
      {
        workspaceRoot: "C:/repo",
        openWorkspaceDirectory,
        openWorkspaceArtifact,
      },
    );

    await user.click(
      screen.getByRole("button", { name: /打开路径 cli|Open path cli/ }),
    );
    await user.click(
      screen.getByRole("button", { name: /打开路径 docs|Open path docs/ }),
    );
    expect(openWorkspaceDirectory).not.toHaveBeenCalled();
    expect(openWorkspaceArtifact.mock.calls).toEqual([
      ["cli", undefined, undefined],
      ["docs", undefined, undefined],
    ]);
  });

  it("links artifacts from the real PowerShell Name Mode output", async () => {
    const user = userEvent.setup();
    const openWorkspaceDirectory = vi.fn();
    const openWorkspaceFile = vi.fn();
    const output =
      "Name       Mode\n----       ----\n.codex     d-----\npackages   d-----\ninstall    -a----\nLICENSE    -a----";
    await renderMessageList(
      [
        turn(
          "turn-1",
          [searchTool(output)],
          "**目录：** `.codex` `packages`\n\n**文件：** `install` `LICENSE`",
        ),
      ],
      {
        workspaceRoot: "C:/repo",
        openWorkspaceDirectory,
        openWorkspaceFile,
      },
    );

    await user.click(
      screen.getByRole("button", {
        name: /打开路径 \.codex|Open path \.codex/,
      }),
    );
    await user.click(
      screen.getByRole("button", {
        name: /打开文件 install|Open file install/,
      }),
    );
    expect(openWorkspaceDirectory).toHaveBeenCalledWith(".codex");
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "install",
      undefined,
      undefined,
    );
  });

  it("links comma-separated prose from ANSI PowerShell tables", async () => {
    const user = userEvent.setup();
    const openWorkspaceDirectory = vi.fn();
    const openWorkspaceFile = vi.fn();
    const output =
      "\u001b[32;1mMode \u001b[0m\u001b[32;1m Length\u001b[0m\u001b[32;1m Name\u001b[0m\n" +
      "d----        packages\n-a--- 14150  install\n-a--- 1086   LICENSE";
    await renderMessageList(
      [
        turn(
          "turn-1",
          [directoryListingTool(output)],
          "**目录：**\n- packages,\n\n**文件：**\n- install, LICENSE",
        ),
      ],
      { openWorkspaceDirectory, openWorkspaceFile },
    );

    await user.click(
      screen.getByRole("button", {
        name: /打开路径 packages|Open path packages/,
      }),
    );
    await user.click(
      screen.getByRole("button", {
        name: /打开文件 install|Open file install/,
      }),
    );
    expect(openWorkspaceDirectory).toHaveBeenCalledWith("packages");
    expect(openWorkspaceFile).toHaveBeenCalledWith(
      "install",
      undefined,
      undefined,
    );
  });

  it("links individual tokens inside aligned plaintext fences", async () => {
    const user = userEvent.setup();
    const openWorkspaceArtifact = vi.fn();
    const output =
      ".github        .husky       packages\ninstall        LICENSE      README.md";
    await renderMessageList(
      [
        turn(
          "turn-1",
          [directoryListingTool(output)],
          "```text\n.github  .husky  packages\ninstall  LICENSE  README.*.md  README.md\n```",
        ),
      ],
      { openWorkspaceArtifact },
    );

    await user.click(
      screen.getByRole("button", {
        name: /打开路径 \.github|Open path \.github/,
      }),
    );
    await user.click(
      screen.getByRole("button", {
        name: /打开路径 LICENSE|Open path LICENSE/,
      }),
    );
    expect(openWorkspaceArtifact.mock.calls).toEqual([
      [".github", undefined, undefined],
      ["LICENSE", undefined, undefined],
    ]);
    expect(screen.getByTestId("chat-path-list")).toHaveTextContent(
      "README.*.md",
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
