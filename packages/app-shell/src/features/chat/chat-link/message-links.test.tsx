import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { PlatformProvider } from "@ora/platform";
import type { ChatToolCall, ChatTurn } from "@ora/chat";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../../i18n/i18n";
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

describe("session-wide chat links", () => {
  it("opens a later mention of an earlier edited file in Changes", async () => {
    const user = userEvent.setup();
    const openDiff = vi.fn();
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TaskChangesNavigationProvider
            onOpenDiff={openDiff}
            onOpenWorkspaceFile={vi.fn()}
          >
            <MessageList
              taskId="task-1"
              turns={[
                turn("turn-1", [editTool("src/main.rs")]),
                turn("turn-2", [], "Updated `src/main.rs`"),
              ]}
              userName="Ada"
              isResponding={false}
            />
          </TaskChangesNavigationProvider>
        </AppI18nProvider>
      </PlatformProvider>,
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
    render(
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TaskChangesNavigationProvider
            onOpenDiff={vi.fn()}
            onOpenWorkspaceFile={openWorkspaceFile}
          >
            <MessageList
              taskId="task-1"
              turns={[
                turn("turn-1", [readTool("src/lib.rs")], "See `src/lib.rs`"),
              ]}
              userName="Ada"
              isResponding={false}
            />
          </TaskChangesNavigationProvider>
        </AppI18nProvider>
      </PlatformProvider>,
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
});
