import { createElement, StrictMode, type ReactNode } from "react";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  ChatContent,
  ChatMessage,
  ChatThought,
  ChatToolCall,
  ChatTurn,
  ChatTurnItem,
} from "@ora/chat";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@ora/ui";
import { AppI18nProvider } from "../../i18n/i18n";
import { appI18n } from "../../i18n/i18n-instance";
import { ContractsClientContext } from "../../contracts-client-context";
import { ChatStoreContext } from "../../chat-store-context";
import { createChatStore } from "@ora/chat";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { ChatView } from "./chat-view";
import { Composer } from "./composer";
import { ConversationNavigator } from "./conversation-navigator";
import { MessageList } from "./message-list";
import { ToolCallBlock } from "./tool-call-block";
import { useComposerInputStore } from "../../state/stores/composer-input-store";
import {
  resetComposerFileDeliveriesForTests,
  useComposerFileContextStore,
} from "../../state/stores/composer-file-context-store";
import { useDraftSessionsStore } from "../../state/stores/draft-sessions-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import {
  DraftSendAbandonedError,
  noteComposerSendAdoptedSession,
  reparkDraftComposerContent,
  resetComposerSendAdoptionsForTests,
} from "../../state/session-drafts";
import { FILE_MENTION_DEBOUNCE_MS } from "./use-composer-file-mentions";

void appI18n;

function composerText(element: HTMLElement): string {
  return element.dataset.composerText ?? "";
}

/**
 * A quote that fails to insert is reported as an alert banner, never thrown, so
 * "the chip is in the document" is not proof the insert succeeded — a partially
 * applied insert produces both. Every quote test asserts the banner is absent.
 */
function expectNoComposerInjectError(): void {
  expect(screen.queryByRole("alert")).toBeNull();
}

/** Flushes conversation hydrate / chip-inject microtasks scheduled from effects. */
async function flushComposerEffects(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

afterEach(() => {
  vi.unstubAllGlobals();
  // A test that set fake timers and was aborted by its own timeout would
  // otherwise strand every later userEvent await on a clock nobody advances.
  vi.useRealTimers();
  resetComposerSendAdoptionsForTests();
  useDraftSessionsStore.getState().clear();
  useComposerInputStore.getState().reset();
  resetComposerFileDeliveriesForTests();
  useComposerFileContextStore.setState({ pendingByConversation: {} });
});

/** Stale-time-zero QueryClient so tests don't need to wait for refetch intervals. */
function createTestQueryClient() {
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: 0 },
      mutations: { retry: false },
    },
  });
}

/** Renders chat components wrapped in all providers required by the app shell. */
function renderWithI18n(element: ReactNode) {
  const client = createMockClient(createMockClientState());
  const queryClient = createTestQueryClient();
  const chatStore = createChatStore(client.session);
  // A wrapper (rather than a one-off wrapped element) so `rerender` re-applies
  // every provider — the model selector reads the contracts client and the
  // conversation's configuration options on each pass.
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(
      QueryClientProvider,
      { client: queryClient },
      createElement(
        ContractsClientContext.Provider,
        { value: client },
        createElement(
          ChatStoreContext.Provider,
          { value: chatStore },
          createElement(AppI18nProvider, null, children),
        ),
      ),
    );
  return {
    ...render(element, { wrapper }),
    client,
    queryClient,
  };
}

/** Builds one response turn so tests can describe threads without protocol plumbing. */
function turn(
  id: string,
  content: string,
  createdAt: number,
  items: ChatTurnItem[] = [],
  status: ChatTurn["status"] = "completed",
): ChatTurn {
  return {
    id,
    userMessage: {
      kind: "message",
      id: `${id}-user`,
      role: "user",
      content,
      createdAt,
    },
    items,
    status,
    stopReason: null,
    error: null,
    createdAt,
  };
}

/** Builds one assistant text item that lives inside a response turn. */
function assistantItem(
  id: string,
  content: string,
  createdAt: number,
): ChatMessage {
  return { kind: "message", id, role: "assistant", content, createdAt };
}

/** Builds one in-progress tool call so tests can stand in for non-text agent work. */
function toolCallItem(id: string, createdAt: number): ChatToolCall {
  return {
    kind: "toolCall",
    id,
    title: "Read file",
    status: "in_progress",
    content: [],
    locations: [],
    createdAt,
    updatedAt: createdAt,
  };
}

/** Builds one completed file read with a structured path for activity summaries. */
function completedReadItem(
  id: string,
  path: string,
  createdAt: number,
): ChatToolCall {
  return {
    kind: "toolCall",
    id,
    title: `Read ${path}`,
    toolKind: "read",
    status: "completed",
    content: [],
    locations: [{ path }],
    createdAt,
    updatedAt: createdAt,
  };
}

/** Builds one live file read so the header can expose its current structured target. */
function activeReadItem(
  id: string,
  path: string,
  createdAt: number,
): ChatToolCall {
  return {
    ...completedReadItem(id, path, createdAt),
    status: "in_progress",
  };
}

/** Builds one completed edit with a path for change-group coverage. */
function completedEditItem(
  id: string,
  path: string,
  createdAt: number,
): ChatToolCall {
  return {
    kind: "toolCall",
    id,
    title: path,
    toolKind: "edit",
    status: "completed",
    content: [],
    locations: [{ path }],
    createdAt,
    updatedAt: createdAt,
  };
}

/** Builds one completed command for command-group coverage. */
function completedCommandItem(
  id: string,
  title: string,
  createdAt: number,
): ChatToolCall {
  return {
    kind: "toolCall",
    id,
    title,
    toolKind: "execute",
    status: "completed",
    content: [],
    locations: [],
    createdAt,
    updatedAt: createdAt,
  };
}

/** Builds one reasoning update for activity timeline coverage. */
function thoughtItem(
  id: string,
  content: string,
  createdAt: number,
): ChatThought {
  return { kind: "thought", id, content, createdAt };
}

describe("Tool calls", () => {
  it("renders a cancelled tool as settled instead of running", () => {
    renderWithI18n(
      <ToolCallBlock
        tool={{
          ...toolCallItem("tool-1", 100),
          status: "cancelled",
        }}
      />,
    );

    expect(screen.getByText(/已取消|Cancelled/)).toBeVisible();
    expect(screen.queryByText(/执行中|Running/)).toBeNull();
  });
});

describe("Composer", () => {
  it("does not offer the retired Spec mode toggle in the footer", () => {
    renderWithI18n(<Composer onSend={vi.fn()} isResponding={false} />);

    expect(
      screen.queryByRole("button", { name: /Spec 模式|Spec mode/ }),
    ).toBeNull();
    expect(screen.queryByText("Spec 模式")).toBeNull();
    expect(screen.queryByText("Spec mode")).toBeNull();
  });

  it("sends trimmed text with Enter and clears the textarea", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "  hello{Enter}");

    expect(onSend).toHaveBeenCalledWith("hello");
    expect(composerText(textarea)).toBe("");
  });

  it("uses Shift+Enter for a newline without sending", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "first{Shift>}{Enter}{/Shift}second");

    expect(onSend).not.toHaveBeenCalled();
    expect(composerText(textarea)).toBe("first\nsecond");
  });

  it("filters available commands and inserts the keyboard selection without executing it", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    renderWithI18n(
      <Composer
        onSend={onSend}
        isResponding={false}
        availableCommands={[
          { name: "review", description: "Review current changes" },
          {
            name: "test",
            description: "Run the test suite",
            input: { hint: "package" },
          },
        ]}
      />,
    );

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "/");
    expect(
      await screen.findByRole("listbox", { name: "快捷操作" }),
    ).toBeVisible();
    expect(screen.getAllByRole("option")).toHaveLength(3);

    await user.keyboard("{ArrowDown}{Enter}");

    expect(composerText(textarea)).toBe("/test ");
    await waitFor(() => expect(textarea).toHaveFocus());
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    expect(onSend).not.toHaveBeenCalled();
  });

  it("offers roles from the slash palette and inserts them as @ tokens", async () => {
    const user = userEvent.setup();
    renderWithI18n(
      <Composer
        onSend={vi.fn()}
        isResponding={false}
        roles={[
          {
            id: "designer",
            namespace: "ora",
            name: "designer",
            description: "Design roles",
          },
        ]}
      />,
    );

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "/");
    expect(
      await screen.findByRole("listbox", { name: "快捷操作" }),
    ).toBeVisible();
    expect(screen.getByRole("option", { name: "designer" })).toBeVisible();

    await user.click(screen.getByRole("option", { name: "designer" }));

    expect(composerText(textarea)).toBe("@designer ");
    await waitFor(() => expect(textarea).toHaveFocus());
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  });

  it("opens the slash menu after existing prompt text", async () => {
    const user = userEvent.setup();
    renderWithI18n(
      <Composer
        onSend={vi.fn()}
        isResponding={false}
        availableCommands={[
          { name: "review", description: "Review current changes" },
        ]}
      />,
    );

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "hello /");
    expect(
      await screen.findByRole("listbox", { name: "快捷操作" }),
    ).toBeVisible();
  });

  it("keeps the keyboard-selected command inside the visible list", async () => {
    const user = userEvent.setup();
    const scrollIntoView = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: scrollIntoView,
    });
    renderWithI18n(
      <Composer
        onSend={() => {}}
        isResponding={false}
        availableCommands={Array.from({ length: 12 }, (_, index) => ({
          name: `command-${index}`,
          description: `Command ${index}`,
        }))}
      />,
    );

    await user.type(screen.getByRole("textbox"), "/");
    await user.click(
      await screen.findByRole("button", { name: "显示另外 7 项" }),
    );
    await user.keyboard(
      "{ArrowDown}{ArrowDown}{ArrowDown}{ArrowDown}{ArrowDown}{ArrowDown}{ArrowDown}{ArrowDown}",
    );

    expect(screen.getAllByRole("option")[8]).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(scrollIntoView).toHaveBeenLastCalledWith({ block: "nearest" });

    await user.click(screen.getByRole("button", { name: "收起" }));

    expect(screen.getAllByRole("option")).toHaveLength(6);
    expect(screen.getByRole("button", { name: "显示另外 7 项" })).toBeVisible();
  });

  it("does not scroll the action menu when the pointer retargets a row", async () => {
    const user = userEvent.setup();
    const scrollIntoView = vi.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      value: scrollIntoView,
    });
    renderWithI18n(
      <Composer
        onSend={() => {}}
        isResponding={false}
        availableCommands={Array.from({ length: 12 }, (_, index) => ({
          name: `command-${index}`,
          description: `Command ${index}`,
        }))}
      />,
    );

    await user.type(screen.getByRole("textbox"), "/");
    const options = await screen.findAllByRole("option");
    scrollIntoView.mockClear();
    await user.hover(options[3]!);

    expect(options[3]).toHaveAttribute("aria-selected", "true");
    expect(scrollIntoView).not.toHaveBeenCalled();
  });

  it("opens the same grouped palette from plus and inserts a selected skill", async () => {
    const user = userEvent.setup();
    renderWithI18n(
      <Composer
        onSend={() => {}}
        isResponding={false}
        skills={[
          {
            id: "skill-1",
            namespace: "local",
            name: "code-review",
            description: "Review the current diff",
            source: { kind: "local" } as const,
            availability: "available",
          },
        ]}
        availableCommands={[{ name: "test", description: "Run tests" }]}
      />,
    );

    await user.click(screen.getByRole("button", { name: "打开快捷操作" }));

    expect(screen.getByText("Skills")).toBeVisible();
    expect(screen.getByText("Commands")).toBeVisible();
    await user.click(screen.getByRole("option", { name: "code-review" }));

    expect(composerText(screen.getByRole("textbox"))).toBe("$code-review ");
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  });

  it("hides unavailable skills from the composer palette", async () => {
    const user = userEvent.setup();
    renderWithI18n(
      <Composer
        onSend={() => {}}
        isResponding={false}
        skills={[
          {
            id: "skill-1",
            namespace: "local",
            name: "code-review",
            description: "Review the current diff",
            source: { kind: "local" } as const,
            availability: "available",
          },
          {
            id: "skill-2",
            namespace: "local",
            name: "missing-skill",
            description: "Lost package",
            source: { kind: "local" } as const,
            availability: "unavailable",
          },
        ]}
        availableCommands={[]}
      />,
    );

    await user.click(screen.getByRole("button", { name: "打开快捷操作" }));

    expect(screen.getByRole("option", { name: "code-review" })).toBeVisible();
    expect(
      screen.queryByRole("option", { name: "missing-skill" }),
    ).not.toBeInTheDocument();
  });

  it("previews a selected image and sends it as ACP image content", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    const view = renderWithI18n(
      <Composer onSend={onSend} isResponding={false} />,
    );
    const fileInput = view.container.querySelector(
      'input[type="file"]',
    ) as HTMLInputElement;

    await user.upload(
      fileInput,
      new File(["hello"], "diagram.png", { type: "image/png" }),
    );
    expect(
      await screen.findByRole("img", { name: "diagram.png" }),
    ).toBeVisible();

    await user.type(screen.getByRole("textbox"), "inspect this{Enter}");

    expect(onSend).toHaveBeenCalledWith("inspect this", [
      {
        data: "aGVsbG8=",
        mimeType: "image/png",
        uri: "diagram.png",
      },
    ]);
  });

  it("pastes a clipboard image into the attachment list", async () => {
    const onSend = vi.fn();
    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    const image = new File(["clipboard"], "clipboard.png", {
      type: "image/png",
    });

    fireEvent.paste(textarea, {
      clipboardData: {
        files: [image],
        getData: () => "",
      },
    });

    expect(
      await screen.findByRole("img", { name: "clipboard.png" }),
    ).toBeVisible();
    expect(composerText(textarea)).toBe("");
    expect(onSend).not.toHaveBeenCalled();
  });

  it("pastes clipboard text alongside an image instead of dropping the text", async () => {
    const onSend = vi.fn();
    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    const image = new File(["clipboard"], "clipboard.png", {
      type: "image/png",
    });

    fireEvent.paste(textarea, {
      clipboardData: {
        files: [image],
        getData: (type: string) =>
          type === "text/plain" ? "**pasted** note" : "",
      },
    });

    expect(
      await screen.findByRole("img", { name: "clipboard.png" }),
    ).toBeVisible();
    await waitFor(() => {
      expect(composerText(textarea)).toContain("pasted");
      expect(textarea.querySelector("strong, b")).not.toBeNull();
    });
    expect(onSend).not.toHaveBeenCalled();
  });

  it("restores unsent text when switching between persisted sessions", async () => {
    const user = userEvent.setup();
    useComposerInputStore.getState().reset();
    useWorkspaceSelectionStore
      .getState()
      .selectSession("session-a", "task-1", "project-1");

    renderWithI18n(<Composer onSend={vi.fn()} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "parked on A");

    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-b", "task-1", "project-1");
    });
    await flushComposerEffects();
    expect(composerText(textarea)).toBe("");

    await user.type(textarea, "on B");
    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-a", "task-1", "project-1");
    });
    await flushComposerEffects();
    expect(composerText(textarea)).toBe("parked on A");

    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-b", "task-1", "project-1");
    });
    await flushComposerEffects();
    expect(composerText(textarea)).toBe("on B");
  });

  it("does not park the leaving session's editor onto the next session key before hydrate", async () => {
    const user = userEvent.setup();
    useComposerInputStore.getState().reset();
    useWorkspaceSelectionStore
      .getState()
      .selectSession("session-a", "task-1", "project-1");

    renderWithI18n(<Composer onSend={vi.fn()} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "content on A");

    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-b", "task-1", "project-1");
    });

    expect(useComposerInputStore.getState().byKey["session-a"]?.text).toBe(
      "content on A",
    );
    expect(useComposerInputStore.getState().byKey["session-b"]).toBeUndefined();

    await flushComposerEffects();
    expect(composerText(textarea)).toBe("");
  });

  it("restores file and skill chips (not inline code) when switching sessions", async () => {
    useComposerInputStore.getState().reset();
    const parkedDoc = {
      type: "doc",
      content: [
        {
          type: "paragraph",
          content: [
            {
              type: "promptToken",
              attrs: { kind: "skill", name: "code-review" },
            },
            { type: "text", text: " " },
            {
              type: "composerFile",
              attrs: {
                path: "src/app.ts",
                startLine: null,
                endLine: null,
                kind: "file",
              },
            },
            { type: "text", text: " " },
            {
              type: "composerFile",
              attrs: {
                path: "src",
                startLine: null,
                endLine: null,
                kind: "directory",
              },
            },
            { type: "text", text: " " },
            {
              type: "promptToken",
              attrs: { kind: "command", name: "test" },
            },
            { type: "text", text: " " },
          ],
        },
      ],
    };
    useComposerInputStore.getState().setInput("session-a", {
      text: "$code-review `src/app.ts` `src` /test ",
      images: [],
      doc: parkedDoc,
    });
    useWorkspaceSelectionStore
      .getState()
      .selectSession("session-b", "task-1", "project-1");

    renderWithI18n(<Composer onSend={vi.fn()} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await flushComposerEffects();
    expect(composerText(textarea)).toBe("");

    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-a", "task-1", "project-1");
    });
    await flushComposerEffects();

    await waitFor(() => {
      expect(
        textarea.querySelector("[data-prompt-token='skill']"),
      ).not.toBeNull();
      expect(
        textarea.querySelector("[data-composer-file='src/app.ts']"),
      ).not.toBeNull();
      expect(textarea.querySelector("code")).toBeNull();
      const dirChip = textarea.querySelector("[data-composer-file='src']");
      expect(dirChip).not.toBeNull();
      expect(dirChip).toHaveAttribute("data-kind", "directory");
      expect(
        textarea.querySelector("[data-prompt-token='command']"),
      ).not.toBeNull();
    });
  });

  it("rebuilds path and $skill chips from parked text when TipTap doc is missing", async () => {
    useComposerInputStore.getState().reset();
    useComposerInputStore.getState().setInput("session-a", {
      text: "see `src/app.ts` and $code-review",
      images: [],
    });
    useWorkspaceSelectionStore
      .getState()
      .selectSession("session-b", "task-1", "project-1");

    renderWithI18n(<Composer onSend={vi.fn()} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await flushComposerEffects();

    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-a", "task-1", "project-1");
    });
    await flushComposerEffects();

    await waitFor(() => {
      expect(
        textarea.querySelector("[data-composer-file='src/app.ts']"),
      ).not.toBeNull();
      expect(
        textarea.querySelector("[data-prompt-token='skill']"),
      ).not.toBeNull();
      expect(textarea.querySelector("code")).toBeNull();
    });
  });

  it("keeps parked images isolated when switching sessions", async () => {
    const user = userEvent.setup();
    useComposerInputStore.getState().reset();
    useWorkspaceSelectionStore
      .getState()
      .selectSession("session-a", "task-1", "project-1");

    renderWithI18n(<Composer onSend={vi.fn()} isResponding={false} />);
    const fileInput = document.querySelector(
      'input[type="file"]',
    ) as HTMLInputElement;

    await user.upload(
      fileInput,
      new File(["a-bytes"], "a.png", { type: "image/png" }),
    );
    expect(await screen.findByRole("img", { name: "a.png" })).toBeVisible();

    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-b", "task-1", "project-1");
    });
    await flushComposerEffects();
    expect(screen.queryByRole("img", { name: "a.png" })).toBeNull();

    await user.upload(
      fileInput,
      new File(["b-bytes"], "b.png", { type: "image/png" }),
    );
    expect(await screen.findByRole("img", { name: "b.png" })).toBeVisible();

    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-a", "task-1", "project-1");
    });
    await flushComposerEffects();
    expect(await screen.findByRole("img", { name: "a.png" })).toBeVisible();
    expect(screen.queryByRole("img", { name: "b.png" })).toBeNull();
  });

  it("hydrates independently when switching between task-only surfaces", async () => {
    const user = userEvent.setup();
    useComposerInputStore.getState().reset();
    useWorkspaceSelectionStore.getState().selectTask("task-a", "project-1");

    renderWithI18n(<Composer onSend={vi.fn()} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await flushComposerEffects();
    await user.type(textarea, "parked on task A");

    act(() => {
      useWorkspaceSelectionStore.getState().selectTask("task-b", "project-1");
    });
    await flushComposerEffects();
    expect(composerText(textarea)).toBe("");
    await user.type(textarea, "task B");

    act(() => {
      useWorkspaceSelectionStore.getState().selectTask("task-a", "project-1");
    });
    await flushComposerEffects();
    expect(composerText(textarea)).toBe("parked on task A");
  });

  it("keeps typed draft text when leaving and returning to a draft", async () => {
    const user = userEvent.setup();
    useComposerInputStore.getState().reset();
    useDraftSessionsStore.getState().clear();
    const draftId = useDraftSessionsStore.getState().ensureEmptyDraft({
      projectId: "project-1",
      taskId: null,
    });
    useWorkspaceSelectionStore
      .getState()
      .selectDraft(draftId, null, "project-1");

    renderWithI18n(<Composer onSend={vi.fn()} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await flushComposerEffects();
    await user.type(textarea, "draft note");

    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-a", "task-1", "project-1");
    });
    await flushComposerEffects();
    expect(composerText(textarea)).toBe("");
    expect(
      useDraftSessionsStore.getState().drafts.find((d) => d.id === draftId)
        ?.text,
    ).toBe("draft note");

    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectDraft(draftId, null, "project-1");
    });
    await flushComposerEffects();
    expect(composerText(textarea)).toBe("draft note");
  });

  it("clears parked input after send so a later return stays empty", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    useComposerInputStore.getState().reset();
    useWorkspaceSelectionStore
      .getState()
      .selectSession("session-a", "task-1", "project-1");

    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await flushComposerEffects();
    await user.type(textarea, "send me{Enter}");
    expect(onSend).toHaveBeenCalledWith("send me");
    expect(useComposerInputStore.getState().byKey["session-a"]).toBeUndefined();

    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-b", "task-1", "project-1");
    });
    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-a", "task-1", "project-1");
    });
    await flushComposerEffects();
    expect(composerText(textarea)).toBe("");
  });

  it("restores composer text when onSend rejects on the same surface", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn(() =>
      Promise.reject(new Error("session start failed")),
    );
    useComposerInputStore.getState().reset();
    useDraftSessionsStore.getState().clear();
    const draftId = useDraftSessionsStore.getState().ensureEmptyDraft({
      projectId: "project-1",
      taskId: null,
    });
    useWorkspaceSelectionStore
      .getState()
      .selectDraft(draftId, null, "project-1");

    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "try again{Enter}");

    expect(onSend).toHaveBeenCalledWith("try again");
    await waitFor(() => expect(composerText(textarea)).toBe("try again"));
    expect(
      useComposerInputStore.getState().byKey[`draft:${draftId}`]?.text,
    ).toBe("try again");
  });

  it("restores composer text when onSend throws synchronously", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn(() => {
      throw new Error("sync failure");
    });
    useComposerInputStore.getState().reset();
    useWorkspaceSelectionStore
      .getState()
      .selectSession("session-a", "task-1", "project-1");

    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "restore me{Enter}");

    expect(onSend).toHaveBeenCalledWith("restore me");
    await waitFor(() => expect(composerText(textarea)).toBe("restore me"));
  });

  it("restores composer text when onSend abandons without a hard failure", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn(() => Promise.reject(new DraftSendAbandonedError()));
    useComposerInputStore.getState().reset();
    useDraftSessionsStore.getState().clear();
    const draftId = useDraftSessionsStore.getState().ensureEmptyDraft({
      projectId: "project-1",
      taskId: null,
    });
    useWorkspaceSelectionStore
      .getState()
      .selectDraft(draftId, null, "project-1");

    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "stopped{Enter}");

    await waitFor(() => expect(composerText(textarea)).toBe("stopped"));
  });

  it("does not paint an abandoned send onto a different conversation", async () => {
    const user = userEvent.setup();
    let rejectSend!: (error: Error) => void;
    let sendPromise!: Promise<void>;
    const onSend = vi.fn(() => {
      sendPromise = new Promise<void>((_resolve, reject) => {
        rejectSend = reject;
      });
      return sendPromise;
    });
    useComposerInputStore.getState().reset();
    useDraftSessionsStore.getState().clear();
    const draftA = useDraftSessionsStore.getState().ensureEmptyDraft({
      projectId: "project-1",
      taskId: null,
    });
    useDraftSessionsStore.getState().updateContent(draftA, { text: "keep" });
    const draftB = useDraftSessionsStore.getState().ensureEmptyDraft({
      projectId: "project-1",
      taskId: null,
    });
    useWorkspaceSelectionStore
      .getState()
      .selectDraft(draftA, null, "project-1");

    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await user.clear(textarea);
    await user.type(textarea, "abandoned{Enter}");
    expect(onSend).toHaveBeenCalledOnce();

    // Switch away before the send settles; restore must not touch draft B.
    await act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectDraft(draftB, null, "project-1");
    });
    await flushComposerEffects();
    await waitFor(() => expect(composerText(textarea)).toBe(""));

    // Mirror workspace-view's abandon repark onto the original draft only.
    await act(async () => {
      reparkDraftComposerContent({
        draftId: draftA,
        text: "abandoned",
      });
      rejectSend(new DraftSendAbandonedError());
      await sendPromise.then(
        () => undefined,
        () => undefined,
      );
      await Promise.resolve();
    });

    expect(
      useComposerInputStore.getState().byKey[`draft:${draftA}`]?.text,
    ).toBe("abandoned");
    expect(composerText(textarea)).toBe("");
    expect(
      useComposerInputStore.getState().byKey[`draft:${draftB}`],
    ).toBeUndefined();
  });

  it("keeps TipTap doc on the draft park after abandon away from the send surface", async () => {
    const user = userEvent.setup();
    let rejectSend!: (error: Error) => void;
    let sendPromise!: Promise<void>;
    const onSend = vi.fn(() => {
      sendPromise = new Promise<void>((_resolve, reject) => {
        rejectSend = reject;
      });
      return sendPromise;
    });
    useComposerInputStore.getState().reset();
    useDraftSessionsStore.getState().clear();
    const draftId = useDraftSessionsStore.getState().ensureEmptyDraft({
      projectId: "project-1",
      taskId: null,
    });
    useWorkspaceSelectionStore
      .getState()
      .selectDraft(draftId, null, "project-1");

    renderWithI18n(
      <Composer
        onSend={onSend}
        isResponding={false}
        skills={[
          {
            id: "skill-1",
            namespace: "local",
            name: "code-review",
            description: "Review",
            source: { kind: "local" } as const,
            availability: "available",
          },
        ]}
        availableCommands={[{ name: "test", description: "Run tests" }]}
      />,
    );
    const textarea = screen.getByRole("textbox");
    await user.click(screen.getByRole("button", { name: "打开快捷操作" }));
    await user.click(screen.getByRole("option", { name: "code-review" }));
    expect(
      textarea.querySelector("[data-prompt-token='skill']"),
    ).not.toBeNull();
    await user.type(textarea, "/");
    await user.click(await screen.findByRole("option", { name: "test" }));
    expect(
      textarea.querySelector("[data-prompt-token='command']"),
    ).not.toBeNull();

    await user.keyboard("{Enter}");
    expect(onSend).toHaveBeenCalledOnce();

    await act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-other", "task-1", "project-1");
    });
    await flushComposerEffects();

    await act(async () => {
      reparkDraftComposerContent({
        draftId,
        text: "$code-review /test ",
      });
      rejectSend(new DraftSendAbandonedError());
      await sendPromise.then(
        () => undefined,
        () => undefined,
      );
      await Promise.resolve();
    });

    const parked = useComposerInputStore.getState().byKey[`draft:${draftId}`];
    expect(parked?.doc).toBeDefined();
    const serialized = JSON.stringify(parked?.doc);
    expect(serialized).toContain("promptToken");
    expect(serialized).toContain("code-review");
    expect(serialized).toContain("test");

    await act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectDraft(draftId, null, "project-1");
    });
    await flushComposerEffects();
    await waitFor(() => {
      expect(
        textarea.querySelector("[data-prompt-token='skill']"),
      ).not.toBeNull();
      expect(
        textarea.querySelector("[data-prompt-token='command']"),
      ).not.toBeNull();
    });
  });

  it("does not reopen the @ menu when hydrating a parked @ query", async () => {
    useComposerInputStore.getState().reset();
    useComposerInputStore.getState().setInput("session-a", {
      text: "look at @fi",
      images: [],
      doc: {
        type: "doc",
        content: [
          {
            type: "paragraph",
            content: [{ type: "text", text: "look at @fi" }],
          },
        ],
      },
    });
    useWorkspaceSelectionStore
      .getState()
      .selectSession("session-b", "task-1", "project-1");

    renderWithI18n(<Composer onSend={vi.fn()} isResponding={false} />);
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();

    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-a", "task-1", "project-1");
    });

    await waitFor(() =>
      expect(composerText(screen.getByRole("textbox"))).toBe("look at @fi"),
    );
    await flushComposerEffects();
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  });

  it("injects explorer file chips only into the conversation that queued them", async () => {
    useComposerInputStore.getState().reset();
    useComposerFileContextStore.setState({ pendingByConversation: {} });
    useWorkspaceSelectionStore
      .getState()
      .selectSession("session-a", "task-1", "project-1");

    renderWithI18n(
      <Composer taskId="task-1" onSend={vi.fn()} isResponding={false} />,
    );
    const textarea = screen.getByRole("textbox");
    await flushComposerEffects();

    await act(async () => {
      useComposerFileContextStore.getState().addSelection("session-a", {
        path: "src/only-a.ts",
        startLine: 1,
        endLine: 2,
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    await waitFor(() =>
      expect(
        textarea.querySelector("[data-composer-file='src/only-a.ts']"),
      ).not.toBeNull(),
    );
    expect(
      useComposerFileContextStore.getState().pendingByConversation["session-a"],
    ).toBeUndefined();
    expectNoComposerInjectError();

    act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-b", "task-1", "project-1");
    });
    await flushComposerEffects();
    expect(
      screen
        .getByRole("textbox")
        .querySelector("[data-composer-file='src/only-a.ts']"),
    ).toBeNull();

    act(() => {
      useComposerFileContextStore.getState().addSelection("session-a", {
        path: "src/queued-for-a.ts",
        startLine: 3,
        endLine: 3,
      });
    });
    await flushComposerEffects();
    // Queued for A while viewing B must not land on B.
    expect(
      screen.getByRole("textbox").querySelector("[data-composer-file]"),
    ).toBeNull();
    expect(
      useComposerFileContextStore.getState().pendingByConversation["session-a"],
    ).toEqual([{ path: "src/queued-for-a.ts", startLine: 3, endLine: 3 }]);
    expect(
      useComposerFileContextStore.getState().pendingByConversation["session-b"],
    ).toBeUndefined();
  });

  it("injects each quoted range once under Strict Mode", async () => {
    useComposerInputStore.getState().reset();
    useComposerFileContextStore.setState({ pendingByConversation: {} });
    useWorkspaceSelectionStore
      .getState()
      .selectSession("session-a", "task-1", "project-1");

    renderWithI18n(
      <StrictMode>
        <Composer taskId="task-1" onSend={vi.fn()} isResponding={false} />
      </StrictMode>,
    );
    const textarea = screen.getByRole("textbox");
    await flushComposerEffects();

    await act(async () => {
      useComposerFileContextStore.getState().addSelection("session-a", {
        path: "src/once.ts",
        startLine: 4,
        endLine: 6,
      });
      await Promise.resolve();
      await Promise.resolve();
    });

    await waitFor(() =>
      expect(
        textarea.querySelectorAll("[data-composer-file='src/once.ts']"),
      ).toHaveLength(1),
    );
    expect(
      useComposerFileContextStore.getState().pendingByConversation["session-a"],
    ).toBeUndefined();
    expectNoComposerInjectError();
  });

  it("does not replay a prior quote when a second range is queued", async () => {
    useComposerInputStore.getState().reset();
    useComposerFileContextStore.setState({ pendingByConversation: {} });
    useWorkspaceSelectionStore
      .getState()
      .selectSession("session-a", "task-1", "project-1");

    renderWithI18n(
      <Composer taskId="task-1" onSend={vi.fn()} isResponding={false} />,
    );
    const textarea = screen.getByRole("textbox");
    await flushComposerEffects();

    await act(async () => {
      useComposerFileContextStore.getState().addSelection("session-a", {
        path: "src/first.ts",
        startLine: 1,
        endLine: 1,
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    await waitFor(() =>
      expect(
        textarea.querySelector("[data-composer-file='src/first.ts']"),
      ).not.toBeNull(),
    );

    await act(async () => {
      useComposerFileContextStore.getState().addSelection("session-a", {
        path: "src/second.ts",
        startLine: 2,
        endLine: 2,
      });
      await Promise.resolve();
      await Promise.resolve();
    });
    await waitFor(() =>
      expect(
        textarea.querySelector("[data-composer-file='src/second.ts']"),
      ).not.toBeNull(),
    );
    expect(
      textarea.querySelectorAll("[data-composer-file='src/first.ts']"),
    ).toHaveLength(1);
    expect(
      textarea.querySelectorAll("[data-composer-file='src/second.ts']"),
    ).toHaveLength(1);
    expectNoComposerInjectError();
  });

  it("does not restore an abandoned send over a newer submit on the same surface", async () => {
    const user = userEvent.setup();
    const rejectors: Array<(error: Error) => void> = [];
    const sendPromises: Array<Promise<void>> = [];
    const onSend = vi.fn(() => {
      const promise = new Promise<void>((_resolve, reject) => {
        rejectors.push(reject);
      });
      sendPromises.push(promise);
      return promise;
    });
    useComposerInputStore.getState().reset();
    useDraftSessionsStore.getState().clear();
    const draftId = useDraftSessionsStore.getState().ensureEmptyDraft({
      projectId: "project-1",
      taskId: null,
    });
    useWorkspaceSelectionStore
      .getState()
      .selectDraft(draftId, null, "project-1");

    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "first{Enter}");
    expect(onSend).toHaveBeenCalledTimes(1);

    await user.type(textarea, "second{Enter}");
    expect(onSend).toHaveBeenCalledTimes(2);
    expect(composerText(textarea)).toBe("");

    // First send abandons after the second submit already cleared the composer.
    await act(async () => {
      rejectors[0]!(new DraftSendAbandonedError());
      await sendPromises[0]!.then(
        () => undefined,
        () => undefined,
      );
      await Promise.resolve();
    });
    expect(composerText(textarea)).toBe("");

    // Second send still owns the surface and can restore its own text.
    await act(async () => {
      rejectors[1]!(new Error("session start failed"));
      await sendPromises[1]!.then(
        () => undefined,
        () => undefined,
      );
      await Promise.resolve();
    });
    expect(composerText(textarea)).toBe("second");
  });

  it("does not paint a hard-fail restore onto an unrelated conversation", async () => {
    const user = userEvent.setup();
    let rejectSend!: (error: Error) => void;
    let sendPromise!: Promise<void>;
    const onSend = vi.fn(() => {
      sendPromise = new Promise<void>((_resolve, reject) => {
        rejectSend = reject;
      });
      return sendPromise;
    });
    useComposerInputStore.getState().reset();
    useDraftSessionsStore.getState().clear();
    const draftId = useDraftSessionsStore.getState().ensureEmptyDraft({
      projectId: "project-1",
      taskId: null,
    });
    useWorkspaceSelectionStore
      .getState()
      .selectDraft(draftId, null, "project-1");

    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "lost elsewhere{Enter}");
    expect(onSend).toHaveBeenCalledOnce();

    // First-send adopted the session it created, then the user opened a third chat.
    noteComposerSendAdoptedSession(`draft:${draftId}`, "adopted-session");
    await act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("session-other", "task-1", "project-1");
    });
    await flushComposerEffects();
    await waitFor(() => expect(composerText(textarea)).toBe(""));

    await act(async () => {
      rejectSend(new Error("attach failed"));
      await sendPromise.then(
        () => undefined,
        () => undefined,
      );
      await Promise.resolve();
    });
    expect(composerText(textarea)).toBe("");
    expect(
      useComposerInputStore.getState().byKey["session-other"],
    ).toBeUndefined();
  });

  it("restores a hard failure onto the session the draft adopted", async () => {
    const user = userEvent.setup();
    let rejectSend!: (error: Error) => void;
    let sendPromise!: Promise<void>;
    const onSend = vi.fn(() => {
      sendPromise = new Promise<void>((_resolve, reject) => {
        rejectSend = reject;
      });
      return sendPromise;
    });
    useComposerInputStore.getState().reset();
    useDraftSessionsStore.getState().clear();
    const draftId = useDraftSessionsStore.getState().ensureEmptyDraft({
      projectId: "project-1",
      taskId: null,
    });
    useWorkspaceSelectionStore
      .getState()
      .selectDraft(draftId, null, "project-1");

    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);
    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "keep on session{Enter}");

    noteComposerSendAdoptedSession(`draft:${draftId}`, "adopted-session");
    await act(() => {
      useWorkspaceSelectionStore
        .getState()
        .selectSession("adopted-session", "task-1", "project-1");
    });
    await flushComposerEffects();
    await waitFor(() => expect(composerText(textarea)).toBe(""));

    await act(async () => {
      rejectSend(new Error("attach failed"));
      await sendPromise.then(
        () => undefined,
        () => undefined,
      );
      await Promise.resolve();
    });
    expect(composerText(textarea)).toBe("keep on session");
  });

  it("mentions a workspace file with @ and inserts a path chip", async () => {
    const user = userEvent.setup();
    const client = createMockClient(createMockClientState());
    client.fileSystem.searchWorkspace = async ({ query }) => {
      const paths = ["src/app.ts", "src/lib/util.ts", "README.md"].filter(
        (path) => path.toLowerCase().includes(query.toLowerCase()),
      );
      return {
        results: paths.map((path) => ({ kind: "file" as const, path })),
        truncated: false,
      };
    };
    client.fileSystem.listWorkspaceDirectory = async () => ({
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

    const queryClient = createTestQueryClient();
    const chatStore = createChatStore(client.session);
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          ContractsClientContext.Provider,
          { value: client },
          createElement(
            ChatStoreContext.Provider,
            { value: chatStore },
            createElement(AppI18nProvider, null, children),
          ),
        ),
      );
    render(<Composer taskId="task-1" onSend={vi.fn()} isResponding={false} />, {
      wrapper,
    });

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "@app");
    expect(
      await screen.findByRole("listbox", { name: "快捷操作" }),
    ).toBeVisible();
    expect(
      await screen.findByRole("option", { name: /app\.ts/ }),
    ).toBeVisible();
    expect(screen.queryByText("Plugins")).toBeNull();

    await user.click(screen.getByRole("option", { name: /app\.ts/ }));

    expect(composerText(textarea)).toContain("`src/app.ts`");
    expect(
      textarea.querySelector("[data-composer-file='src/app.ts']"),
    ).not.toBeNull();
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  });

  it("keeps plugins on the plus menu after @ became file mentions", async () => {
    const user = userEvent.setup();
    renderWithI18n(<Composer onSend={vi.fn()} isResponding={false} />);

    await user.type(screen.getByRole("textbox"), "@");
    expect(
      await screen.findByRole("listbox", { name: "快捷操作" }),
    ).toBeVisible();
    expect(screen.queryByText("Plugins")).toBeNull();
    expect(
      screen.getByText(
        /选择项目后即可用 @ 引用文件或文件夹|Select a project to mention files or folders/,
      ),
    ).toBeVisible();

    await user.keyboard("{Escape}");
    await user.click(screen.getByRole("button", { name: "打开快捷操作" }));
    expect(screen.getByText("Actions")).toBeVisible();
  });

  it("mentions a project file with @ when no task is selected yet", async () => {
    const user = userEvent.setup();
    const client = createMockClient(createMockClientState());
    client.fileSystem.listProjectDirectory = async () => ({
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
    client.fileSystem.searchProject = async ({ query }) => ({
      results: ["src/draft.ts"]
        .filter((path) => path.toLowerCase().includes(query.toLowerCase()))
        .map((path) => ({ kind: "file" as const, path })),
      truncated: false,
    });

    const queryClient = createTestQueryClient();
    const chatStore = createChatStore(client.session);
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          ContractsClientContext.Provider,
          { value: client },
          createElement(
            ChatStoreContext.Provider,
            { value: chatStore },
            createElement(AppI18nProvider, null, children),
          ),
        ),
      );
    render(
      <Composer projectId="project-1" onSend={vi.fn()} isResponding={false} />,
      { wrapper },
    );

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "@draft");
    await user.click(await screen.findByRole("option", { name: /draft\.ts/ }));

    expect(composerText(textarea)).toContain("`src/draft.ts`");
    expect(
      textarea.querySelector("[data-composer-file='src/draft.ts']"),
    ).not.toBeNull();
  });

  it("mentions a workspace folder with @ and inserts a path chip", async () => {
    const user = userEvent.setup();
    const client = createMockClient(createMockClientState());
    client.fileSystem.listWorkspaceDirectory = async () => ({
      path: "",
      entries: [
        {
          name: "src",
          path: "src",
          kind: "directory",
          isSymbolicLink: false,
        },
        {
          name: "README.md",
          path: "README.md",
          kind: "file",
          isSymbolicLink: false,
        },
      ],
    });
    client.fileSystem.searchWorkspace = async ({ query }) => ({
      results: ["src/app.ts"]
        .filter((path) => path.toLowerCase().includes(query.toLowerCase()))
        .map((path) => ({ kind: "file" as const, path })),
      truncated: false,
    });

    const queryClient = createTestQueryClient();
    const chatStore = createChatStore(client.session);
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          ContractsClientContext.Provider,
          { value: client },
          createElement(
            ChatStoreContext.Provider,
            { value: chatStore },
            createElement(AppI18nProvider, null, children),
          ),
        ),
      );
    render(<Composer taskId="task-1" onSend={vi.fn()} isResponding={false} />, {
      wrapper,
    });

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "@");
    const folderOption = await screen.findByRole("option", { name: /^src/ });
    expect(folderOption).toBeVisible();
    await user.click(folderOption);

    expect(composerText(textarea)).toContain("`src`");
    expect(textarea.querySelector("[data-composer-file='src']")).not.toBeNull();
  });

  it("keeps root file hits during debounce but disables selection until search settles", async () => {
    const user = userEvent.setup();
    const client = createMockClient(createMockClientState());
    client.fileSystem.listWorkspaceDirectory = async () => ({
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
    let releaseSearch: (value: {
      results: Array<{ kind: "file"; path: string }>;
      truncated: boolean;
    }) => void = () => {};
    client.fileSystem.searchWorkspace = () =>
      new Promise((resolve) => {
        releaseSearch = resolve;
      });

    const queryClient = createTestQueryClient();
    const chatStore = createChatStore(client.session);
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          ContractsClientContext.Provider,
          { value: client },
          createElement(
            ChatStoreContext.Provider,
            { value: chatStore },
            createElement(AppI18nProvider, null, children),
          ),
        ),
      );
    render(<Composer taskId="task-1" onSend={vi.fn()} isResponding={false} />, {
      wrapper,
    });

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "@");
    const rootOption = await screen.findByRole("option", {
      name: /README\.md/,
    });
    expect(rootOption).toBeVisible();
    expect(rootOption).toBeEnabled();

    await user.type(textarea, "app");
    expect(
      await screen.findByRole("option", { name: /README\.md/ }),
    ).toHaveAttribute("aria-disabled", "true");
    expect(screen.queryByText(/正在搜索文件|Searching files/)).toBeNull();

    await act(async () => {
      await new Promise((resolve) =>
        setTimeout(resolve, FILE_MENTION_DEBOUNCE_MS + 40),
      );
    });
    // Fetch keeps prior hits visible (locked) with searching chrome above.
    expect(screen.getByRole("option", { name: /README\.md/ })).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    expect(screen.getByText(/正在搜索文件|Searching files/)).toBeVisible();

    await act(async () => {
      releaseSearch({
        results: [{ kind: "file", path: "src/app.ts" }],
        truncated: false,
      });
    });
    const appOption = await screen.findByRole("option", { name: /app\.ts/ });
    expect(appOption).toHaveAttribute("aria-disabled", "false");
  });

  it("shows an error when workspace file search fails", async () => {
    const user = userEvent.setup();
    const client = createMockClient(createMockClientState());
    client.fileSystem.searchWorkspace = async () => {
      throw new Error("search failed");
    };

    const queryClient = createTestQueryClient();
    const chatStore = createChatStore(client.session);
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          ContractsClientContext.Provider,
          { value: client },
          createElement(
            ChatStoreContext.Provider,
            { value: chatStore },
            createElement(AppI18nProvider, null, children),
          ),
        ),
      );
    render(<Composer taskId="task-1" onSend={vi.fn()} isResponding={false} />, {
      wrapper,
    });

    await user.type(screen.getByRole("textbox"), "@app");
    expect(
      await screen.findByText(
        /无法搜索工作区文件|Could not search workspace files/,
      ),
    ).toBeVisible();
    expect(screen.queryByRole("option")).toBeNull();
  });

  it("keeps typing after an @ file chip inserted mid-prompt", async () => {
    const user = userEvent.setup();
    const client = createMockClient(createMockClientState());
    client.fileSystem.searchWorkspace = async () => ({
      results: [{ kind: "file", path: "src/mid.ts" }],
      truncated: false,
    });

    const queryClient = createTestQueryClient();
    const chatStore = createChatStore(client.session);
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          ContractsClientContext.Provider,
          { value: client },
          createElement(
            ChatStoreContext.Provider,
            { value: chatStore },
            createElement(AppI18nProvider, null, children),
          ),
        ),
      );
    render(<Composer taskId="task-1" onSend={vi.fn()} isResponding={false} />, {
      wrapper,
    });

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "check @mid");
    await user.click(await screen.findByRole("option", { name: /mid\.ts/ }));
    await user.keyboard("please");

    // Chip serialization already inserts a trailing space after the atom.
    expect(composerText(textarea)).toBe("check `src/mid.ts` please");
  });
});

describe("Composer send gating", () => {
  const SEND_HINT = /请先选择一个可用的Agent模型|Pick an available agent/;
  const sendButton = () =>
    screen.getByRole("button", { name: /发送消息|Send message/ });

  it("keeps typing and attachments available while the send button is gated", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    renderWithI18n(
      <TooltipProvider>
        <Composer
          onSend={onSend}
          isResponding={false}
          sendDisabledHint="hint"
        />
      </TooltipProvider>,
    );

    // Dimmed via aria-disabled rather than disabled, so the control stays in
    // the tab order and keyboard users can reach the explanation.
    expect(sendButton()).toHaveAttribute("aria-disabled", "true");
    expect(sendButton()).toBeEnabled();
    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "hello");

    expect(composerText(textarea)).toBe("hello");
    // The gated control is focusable, so a direct click reaches submit and has
    // to be absorbed by the canSend guard rather than by the disabled state.
    await user.click(sendButton());
    expect(onSend).not.toHaveBeenCalled();
  });

  it("does not send on Enter while gated and parks the typed text", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    renderWithI18n(
      <TooltipProvider>
        <Composer
          onSend={onSend}
          isResponding={false}
          sendDisabledHint="hint"
        />
      </TooltipProvider>,
    );

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "hello{Enter}");

    expect(onSend).not.toHaveBeenCalled();
    expect(composerText(textarea)).toBe("hello");
  });

  it("explains the gate on hover over the gated send button", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    renderWithI18n(
      <TooltipProvider>
        <Composer
          onSend={onSend}
          isResponding={false}
          sendDisabledHint="请先选择一个可用的Agent模型"
        />
      </TooltipProvider>,
    );

    await user.hover(sendButton());
    expect(await screen.findByText(SEND_HINT)).toBeVisible();
  });

  it("explains the gate on keyboard focus of the gated send button", async () => {
    const onSend = vi.fn();
    renderWithI18n(
      <TooltipProvider>
        <Composer
          onSend={onSend}
          isResponding={false}
          sendDisabledHint="请先选择一个可用的Agent模型"
        />
      </TooltipProvider>,
    );

    // Hover is unavailable to a keyboard-only user; tabbing to the dimmed
    // button is what surfaces the hint there. The gated control keeps its tab
    // stop precisely so this path exists.
    act(() => sendButton().focus());
    expect(await screen.findByText(SEND_HINT)).toBeVisible();
  });

  it("pins the hint open when a gated Enter refuses to send, then fades it", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    renderWithI18n(
      <TooltipProvider>
        <Composer
          onSend={onSend}
          isResponding={false}
          sendDisabledHint="请先选择一个可用的Agent模型"
        />
      </TooltipProvider>,
    );

    await user.type(screen.getByRole("textbox"), "hello{Enter}");

    expect(onSend).not.toHaveBeenCalled();
    // The refusal is announced at the button, not swallowed silently — the
    // keyboard send gets the same feedback a hover on the button would give.
    expect(await screen.findByText(SEND_HINT)).toBeVisible();
    // The pin is transient: the bubble retires itself after the pin window.
    await waitFor(() => expect(screen.queryByText(SEND_HINT)).toBeNull(), {
      timeout: 4_000,
    });
  });

  it("retires a pinned hint the moment the gate lifts", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    const view = renderWithI18n(
      <TooltipProvider>
        <Composer
          onSend={onSend}
          isResponding={false}
          sendDisabledHint="请先选择一个可用的Agent模型"
        />
      </TooltipProvider>,
    );

    await user.type(screen.getByRole("textbox"), "hello{Enter}");
    expect(await screen.findByText(SEND_HINT)).toBeVisible();

    // A stale hint must never linger over a send button that works again, so
    // the pin dies with the gate instead of outliving it to the pin timeout.
    view.rerender(
      <TooltipProvider>
        <Composer onSend={onSend} isResponding={false} />
      </TooltipProvider>,
    );
    await waitFor(() => expect(screen.queryByText(SEND_HINT)).toBeNull());
  });

  it("never attaches the hint to the stop control of a live turn", async () => {
    const user = userEvent.setup();
    const onStop = vi.fn();
    renderWithI18n(
      <TooltipProvider>
        <Composer
          onSend={vi.fn()}
          isResponding
          onStop={onStop}
          sendDisabledHint="hint"
        />
      </TooltipProvider>,
    );

    const stopButton = screen.getByRole("button", {
      name: /正在启动|Starting/,
    });
    expect(stopButton).toBeEnabled();
    await user.hover(stopButton);
    expect(screen.queryByText(SEND_HINT)).toBeNull();

    await user.click(stopButton);
    expect(onStop).toHaveBeenCalledOnce();
  });

  it("lifts the gate on rerender so a later send is not blocked", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    const view = renderWithI18n(
      <TooltipProvider>
        <Composer
          onSend={onSend}
          isResponding={false}
          sendDisabledHint="hint"
        />
      </TooltipProvider>,
    );

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "hello");
    expect(sendButton()).toHaveAttribute("aria-disabled", "true");

    view.rerender(
      <TooltipProvider>
        <Composer onSend={onSend} isResponding={false} />
      </TooltipProvider>,
    );
    expect(sendButton()).toBeEnabled();
    await user.type(textarea, "{Enter}");
    expect(onSend).toHaveBeenCalledWith("hello");
  });

  it("does not send an empty composer on Enter now that submit checks canSend", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);

    await user.type(screen.getByRole("textbox"), "{Enter}");

    expect(onSend).not.toHaveBeenCalled();
  });
});

describe("Structured ACP content", () => {
  it("renders structured resources and previews images with wheel zoom", async () => {
    const user = userEvent.setup();
    const image = {
      type: "image" as const,
      data: "aGVsbG8=",
      mimeType: "image/png",
      uri: "file:///preview.png",
    };
    const items: ChatContent[] = [
      {
        kind: "content",
        id: "audio",
        source: "message",
        content: { type: "audio", data: "aGVsbG8=", mimeType: "audio/mpeg" },
        createdAt: 2,
      },
      {
        kind: "content",
        id: "link",
        source: "message",
        content: {
          type: "resource_link",
          name: "docs",
          title: "ACP docs",
          description: "Protocol reference",
          uri: "https://example.com/acp",
          size: 2048,
        },
        createdAt: 3,
      },
      {
        kind: "content",
        id: "resource",
        source: "message",
        content: {
          type: "resource",
          resource: {
            uri: "file:///notes.txt",
            mimeType: "text/plain",
            text: "embedded notes",
          },
        },
        createdAt: 4,
      },
    ];
    const mediaTurn = turn("media", "show files", 1, items);
    mediaTurn.userMessage.structuredContent = [image];
    const view = renderWithI18n(
      <MessageList turns={[mediaTurn]} userName="Eric" isResponding={false} />,
    );

    const inlineImage = screen.getByRole("img", { name: "preview.png" });
    expect(inlineImage).toHaveAttribute("loading", "lazy");
    expect(inlineImage.closest("a")).toBeNull();
    expect(inlineImage.closest("button")).toBeNull();
    const expandButton = screen.getByRole("button", {
      name: "展开图片 preview.png",
    });
    expect(expandButton).toHaveClass("cursor-pointer");
    expect(view.container.querySelector("audio[controls]")).toHaveAttribute(
      "src",
      "data:audio/mpeg;base64,aGVsbG8=",
    );
    expect(screen.getByRole("link", { name: /ACP docs/ })).toHaveAttribute(
      "href",
      "https://example.com/acp",
    );
    expect(screen.getByText("embedded notes")).toBeVisible();

    await user.click(expandButton);
    expect(screen.getByRole("dialog")).toHaveStyle({
      width: "calc(100vw - 3rem)",
      maxWidth: "88rem",
      height: "calc(100dvh - 3rem)",
    });
    const canvas = screen.getByLabelText("preview.png，缩放 100%");
    const previewImage = document.querySelector('[data-slot="preview-image"]');
    expect(previewImage).not.toBeNull();
    canvas.scrollLeft = 12;
    canvas.scrollTop = 18;
    const wheel = new WheelEvent("wheel", {
      deltaY: -100,
      bubbles: true,
      cancelable: true,
    });
    act(() => expect(canvas.dispatchEvent(wheel)).toBe(false));
    expect(screen.getByLabelText("preview.png，缩放 110%")).toBeVisible();
    expect(previewImage).toHaveStyle({
      transform: "translate(-50%, -50%) translate(0px, 0px) scale(1.1)",
    });
    expect(canvas).toHaveProperty("scrollLeft", 12);
    expect(canvas).toHaveProperty("scrollTop", 18);
    canvas.scrollLeft = 0;
    canvas.scrollTop = 0;
    fireEvent.pointerDown(canvas, {
      button: 0,
      pointerId: 7,
      clientX: 100,
      clientY: 100,
    });
    expect(canvas).toHaveClass("cursor-grabbing");
    fireEvent.pointerMove(canvas, { pointerId: 7, clientX: 60, clientY: 70 });
    expect(previewImage).toHaveStyle({
      transform: "translate(-50%, -50%) translate(-40px, -30px) scale(1.1)",
    });
    fireEvent.pointerUp(canvas, { pointerId: 7, clientX: 60, clientY: 70 });
    expect(canvas).toHaveClass("cursor-grab");
    expect(screen.getByRole("button", { name: "关闭图片预览" })).toBeVisible();
  });
});

describe("ChatView", () => {
  it("disables composition and shows the unavailable Agent session error", () => {
    renderWithI18n(
      <ChatView
        turns={[]}
        userName="Eric"
        isResponding={false}
        error="Agent session unavailable"
        disabled
        onSend={() => {}}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Agent session unavailable",
    );
    expect(screen.getByRole("textbox")).toHaveAttribute(
      "aria-disabled",
      "true",
    );
    expect(screen.getByRole("textbox")).toHaveAttribute(
      "contenteditable",
      "false",
    );
    expect(screen.getAllByRole("button")).toEqual(
      expect.arrayContaining([expect.objectContaining({ disabled: true })]),
    );
  });

  it("keeps the disabled hint shut when the pointer never left the enabled composer", async () => {
    const user = userEvent.setup();
    const view = renderWithI18n(
      <ChatView
        turns={[]}
        userName="Eric"
        isResponding={false}
        error={null}
        onSend={() => {}}
      />,
    );

    // Hover the composer while it has no hint. The real app then slides the
    // composer out from under the pointer, so no pointerleave ever arrives.
    await user.hover(screen.getByRole("textbox"));

    view.rerender(
      <ChatView
        turns={[]}
        userName="Eric"
        isResponding={false}
        error={null}
        disabled
        disabledHint="pick a project"
        onSend={() => {}}
      />,
    );

    expect(screen.queryByText("pick a project")).toBeNull();
  });

  it("renders execution context immediately above the composer surface", () => {
    renderWithI18n(
      <ChatView
        turns={[]}
        userName="Eric"
        isResponding={false}
        error={null}
        contextBar={<span>Ora / frontend</span>}
        onSend={() => {}}
      />,
    );

    const composer = screen
      .getByRole("textbox")
      .closest('[data-slot="composer"]');
    const context = screen
      .getByText("Ora / frontend")
      .closest('[data-slot="composer-context"]');
    expect(composer).not.toBeNull();
    expect(context).not.toBeNull();
    expect(composer?.contains(context)).toBe(false);
    expect(
      context?.nextElementSibling?.querySelector('[data-slot="composer"]'),
    ).toBe(composer);
  });

  it("shows the history loading indicator without the landing copy while a session loads", () => {
    renderWithI18n(
      <ChatView
        turns={[]}
        userName="Eric"
        isResponding={false}
        isLoading
        error={null}
        onSend={() => {}}
      />,
    );

    // Thread layout: the loading status stands in for the yet-to-arrive turns and
    // the landing heading/suggestions are gone, so the composer has slid down.
    expect(
      screen.getByRole("status", { name: /加载历史|Loading history/ }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("heading")).toBeNull();
    expect(screen.queryByRole("textbox")).toBeInTheDocument();
  });

  it("slides the composer down once when a session is selected, not again when its turns land", () => {
    // Same FLIP harness as below: jsdom lacks layout and the Web Animations API.
    let top = 300;
    const rectSpy = vi
      .spyOn(Element.prototype, "getBoundingClientRect")
      .mockImplementation(() => ({ top }) as DOMRect);
    const animate = vi.fn();
    Object.defineProperty(Element.prototype, "animate", {
      configurable: true,
      writable: true,
      value: animate,
    });

    // Landing state: nothing selected, composer centered.
    const view = renderWithI18n(
      <ChatView
        turns={[]}
        userName="Eric"
        isResponding={false}
        error={null}
        onSend={() => {}}
      />,
    );

    // Selecting a session flips it into the loading thread layout: the composer
    // slides down here, before any turn exists.
    top = 800;
    view.rerender(
      <ChatView
        turns={[]}
        userName="Eric"
        isResponding={false}
        isLoading
        error={null}
        onSend={() => {}}
      />,
    );
    expect(animate).toHaveBeenCalledTimes(1);

    // History arriving is not a landing→thread transition, so it must not replay
    // the slide — otherwise the composer animates twice for one selection.
    view.rerender(
      <ChatView
        turns={[turn("turn-1", "hello", 100)]}
        userName="Eric"
        isResponding={false}
        error={null}
        onSend={() => {}}
      />,
    );
    expect(animate).toHaveBeenCalledTimes(1);

    rectSpy.mockRestore();
    Reflect.deleteProperty(Element.prototype, "animate");
  });

  it("slides the same composer node down when the first message arrives", () => {
    // jsdom has no layout and no Web Animations API, so both are stood up here:
    // the rects drive the FLIP delta and the spy captures the resulting keyframes.
    let top = 300;
    const rectSpy = vi
      .spyOn(Element.prototype, "getBoundingClientRect")
      .mockImplementation(() => ({ top }) as DOMRect);
    const animate = vi.fn();
    Object.defineProperty(Element.prototype, "animate", {
      configurable: true,
      writable: true,
      value: animate,
    });

    const view = renderWithI18n(
      <ChatView
        turns={[]}
        userName="Eric"
        isResponding={false}
        error={null}
        onSend={() => {}}
      />,
    );
    const landingComposer = screen.getByRole("textbox");

    top = 800;
    view.rerender(
      <ChatView
        turns={[turn("turn-1", "hello", 100)]}
        userName="Eric"
        isResponding={false}
        error={null}
        onSend={() => {}}
      />,
    );

    // Identity is the whole point: a remounted composer cannot be animated and
    // would drop whatever the user had typed.
    expect(screen.getByRole("textbox")).toBe(landingComposer);
    expect(animate).toHaveBeenCalledWith(
      [{ transform: "translateY(-500px)" }, { transform: "translateY(0)" }],
      expect.objectContaining({ duration: expect.any(Number) }),
    );

    rectSpy.mockRestore();
    Reflect.deleteProperty(Element.prototype, "animate");
  });
});

describe("MessageList", () => {
  it("keeps the ordinary conversation navigator at its viewport threshold", () => {
    renderWithI18n(
      <MessageList
        turns={[
          turn("turn-1", "First", 100, [
            assistantItem("assistant-1", "First answer", 200),
          ]),
        ]}
        userName="Eric"
        isResponding={false}
      />,
    );

    expect(screen.queryByTestId("conversation-anchor-list")).toBeNull();
  });

  it("renders sent user Markdown and copies the source string", async () => {
    const user = userEvent.setup();
    const writeText = vi
      .spyOn(navigator.clipboard, "writeText")
      .mockResolvedValue();
    const source = "**bold** and [Docs](https://example.com)";
    renderWithI18n(
      <MessageList
        turns={[turn("turn-md", source, 100)]}
        userName="Eric"
        isResponding={false}
      />,
    );

    expect(screen.getByText("bold").tagName).toMatch(/^(STRONG|B)$/);
    expect(screen.getByRole("link", { name: "Docs" })).toHaveAttribute(
      "href",
      "https://example.com",
    );
    expect(screen.queryByText(/\*\*bold\*\*/)).toBeNull();

    const row = screen.getByText("bold").closest(".group\\/message");
    expect(row).not.toBeNull();
    await user.hover(row!);
    await user.click(screen.getByRole("button", { name: /复制|Copy/ }));
    expect(writeText).toHaveBeenCalledWith(source);
    writeText.mockRestore();
  });

  it("renders user composer surfaces: highlight, multiline, and headings", () => {
    const source = [
      "# Title",
      "first line",
      "second line",
      "==glow== and ~~gone~~",
    ].join("\n");
    renderWithI18n(
      <MessageList
        turns={[turn("turn-surface", source, 100)]}
        userName="Eric"
        isResponding={false}
      />,
    );

    expect(
      screen.getByRole("heading", { level: 1, name: "Title" }),
    ).not.toBeNull();
    expect(screen.getByText("first line").tagName).toBe("P");
    expect(screen.getByText("second line").tagName).toBe("P");
    expect(screen.getByText("glow").tagName).toBe("MARK");
    expect(screen.getByText("gone").tagName).toBe("DEL");
  });

  it("still shows image-only user messages without a text bubble", () => {
    const mediaTurn = turn("turn-media", "", 100);
    mediaTurn.userMessage.structuredContent = [
      {
        type: "image",
        mimeType: "image/png",
        data: "aaaa",
      },
    ];
    renderWithI18n(
      <MessageList turns={[mediaTurn]} userName="Eric" isResponding={false} />,
    );

    expect(screen.getByRole("img")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /复制|Copy/ })).toBeNull();
  });

  it("divides the thread where the answering model changed", () => {
    renderWithI18n(
      <MessageList
        turns={[turn("turn-1", "First", 100), turn("turn-2", "Second", 200)]}
        modelChanges={[
          {
            id: "change-1",
            afterTurnCount: 1,
            modelName: "Smart",
            createdAt: 150,
          },
        ]}
        userName="Eric"
        isResponding={false}
      />,
    );

    const divider = screen.getByRole("separator", {
      name: /已切换到 Smart|Switched to Smart/,
    });
    const [first, second] = screen.getAllByText(/First|Second/);
    // The divider separates the turns it was recorded between, rather than
    // landing at either end of the thread.
    expect(first!.compareDocumentPosition(divider)).toBe(
      Node.DOCUMENT_POSITION_FOLLOWING,
    );
    expect(second!.compareDocumentPosition(divider)).toBe(
      Node.DOCUMENT_POSITION_PRECEDING,
    );
  });

  it("compresses consecutive reads into a second-level disclosure", async () => {
    const user = userEvent.setup();
    renderWithI18n(
      <MessageList
        turns={[
          turn("turn-1", "Read the reports", 100, [
            completedReadItem("read-1", "a.md", 200),
            completedReadItem("read-2", "b.md", 300),
            completedReadItem("read-3", "c.md", 400),
            completedReadItem("read-4", "d.md", 500),
          ]),
        ]}
        userName="Eric"
        isResponding={false}
      />,
    );

    await user.click(
      screen.getByRole("button", {
        name: /文件读取完成|File reading complete/,
      }),
    );
    const readBatch = screen.getByRole("button", {
      name: /读取 4 个文件|Read 4 files/,
    });
    expect(
      screen.queryByRole("button", { name: /读取\s*a\.md|Read\s*a\.md/ }),
    ).toBeNull();

    await user.click(readBatch);
    expect(
      screen.getByRole("button", { name: /读取\s*a\.md|Read\s*a\.md/ }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: /读取\s*d\.md|Read\s*d\.md/ }),
    ).toBeVisible();
  });

  it("folds reads, edits, and commands into one collapsed phase, each still distinct once expanded", async () => {
    const user = userEvent.setup();
    renderWithI18n(
      <MessageList
        turns={[
          turn("turn-1", "Update and verify the document", 100, [
            completedReadItem("read-1", "report.md", 200),
            completedEditItem("edit-1", "report.md", 300),
            completedEditItem("edit-2", "summary.md", 400),
            completedCommandItem("command-1", "pnpm lint", 500),
            completedCommandItem("command-2", "pnpm test", 600),
          ]),
        ]}
        userName="Eric"
        isResponding={false}
      />,
    );

    expect(
      screen.queryByRole("button", {
        name: /文件读取完成|File reading complete/,
      }),
    ).toBeNull();
    expect(
      screen.queryByRole("button", { name: /已修改 2 个文件|Changed 2 files/ }),
    ).toBeNull();
    expect(
      screen.queryByRole("button", { name: /已执行 2 条命令|Ran 2 commands/ }),
    ).toBeNull();

    await user.click(
      screen.getByRole("button", { name: /已完成的操作|Completed activity/ }),
    );

    expect(
      screen.getByRole("button", {
        name: /文件读取完成|File reading complete/,
      }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: /已修改 2 个文件|Changed 2 files/ }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: /已执行 2 条命令|Ran 2 commands/ }),
    ).toBeVisible();
  });

  it("surfaces a domain-neutral current file while exploration is streaming", () => {
    renderWithI18n(
      <MessageList
        turns={[
          turn(
            "turn-1",
            "Inspect the entry point",
            100,
            [
              thoughtItem(
                "thought-1",
                "Locating the application entry point",
                200,
              ),
              activeReadItem("read-1", "reports/q2.pdf", 300),
            ],
            "streaming",
          ),
        ]}
        userName="Eric"
        isResponding
      />,
    );

    const activity = screen.getByRole("button", {
      name: /正在读取 q2\.pdf|Reading q2\.pdf/,
    });
    expect(activity).toHaveTextContent(
      /1 个文件 · 1 次分析|1 file · 1 analysis step/,
    );
  });

  it("reveals only the live thought suffix and settles it before the next activity", () => {
    const view = renderWithI18n(
      <MessageList
        turns={[
          turn(
            "turn-1",
            "Inspect",
            100,
            [thoughtItem("thought-1", "Checking", 200)],
            "streaming",
          ),
        ]}
        userName="Eric"
        isResponding
      />,
    );

    expect(
      Array.from(
        view.container.querySelectorAll("[data-stream-thought-reveal]"),
      ).map((node) => node.textContent),
    ).toEqual(["Checking"]);

    view.rerender(
      <MessageList
        turns={[
          turn(
            "turn-1",
            "Inspect",
            100,
            [thoughtItem("thought-1", "Checking files", 200)],
            "streaming",
          ),
        ]}
        userName="Eric"
        isResponding
      />,
    );

    expect(
      Array.from(
        view.container.querySelectorAll("[data-stream-thought-reveal]"),
      ).map((node) => node.textContent),
    ).toEqual([" files"]);

    view.rerender(
      <MessageList
        turns={[
          turn(
            "turn-1",
            "Inspect",
            100,
            [
              thoughtItem("thought-1", "Checking files", 200),
              toolCallItem("tool-1", 300),
            ],
            "streaming",
          ),
        ]}
        userName="Eric"
        isResponding
      />,
    );

    expect(
      view.container.querySelector("[data-stream-thought-reveal]"),
    ).toBeNull();
  });

  it("condenses interleaved analysis and file reads into one expandable activity timeline", async () => {
    const user = userEvent.setup();
    renderWithI18n(
      <MessageList
        turns={[
          turn("turn-1", "Inspect the project", 100, [
            thoughtItem("thought-1", "Checking project configuration", 200),
            completedReadItem("read-1", "Cargo.toml", 300),
            thoughtItem("thought-2", "Finding the relevant source", 400),
            completedReadItem("read-2", "src/main.rs", 500),
            assistantItem("assistant-1", "Done", 600),
          ]),
        ]}
        userName="Eric"
        isResponding={false}
      />,
    );

    const activity = screen.getByRole("button", {
      name: /文件读取完成|File reading complete/,
    });
    expect(activity).toHaveTextContent(
      /2 个文件 · 2 次分析|2 files · 2 analysis steps/,
    );
    expect(screen.queryByText("Checking project configuration")).toBeNull();

    await user.click(activity);
    expect(screen.getByText("Cargo.toml")).toBeVisible();
    expect(screen.getByText("main.rs")).toBeVisible();
    const firstThought = screen.getByRole("button", {
      name: /Checking project configuration/,
    });
    const secondThought = screen.getByRole("button", {
      name: /Finding the relevant source/,
    });
    await user.click(firstThought);
    expect(screen.getAllByText("Checking project configuration")).toHaveLength(
      2,
    );

    await user.click(secondThought);
    expect(screen.getAllByText("Checking project configuration")).toHaveLength(
      1,
    );
    expect(screen.getAllByText("Finding the relevant source")).toHaveLength(2);
  });

  it("shows the running indicator while working but hides it as the answer streams", () => {
    const view = renderWithI18n(
      <MessageList
        turns={[turn("turn-1", "hello", 100, [], "streaming")]}
        userName="Eric"
        isResponding
      />,
    );
    // Waiting for the first output: the indicator stands in for the empty turn.
    expect(screen.getByLabelText(/正在运行|is working/)).toBeInTheDocument();

    // Answer body streaming in: the growing text is signal enough, so it hides.
    view.rerender(
      <MessageList
        turns={[
          turn(
            "turn-1",
            "hello",
            100,
            [assistantItem("assistant-1", "Mock", 200)],
            "streaming",
          ),
        ]}
        userName="Eric"
        isResponding
      />,
    );
    expect(
      screen.queryByLabelText(/正在运行|is working/),
    ).not.toBeInTheDocument();

    // Back to working — a tool call trails the text — so the indicator returns.
    view.rerender(
      <MessageList
        turns={[
          turn(
            "turn-1",
            "hello",
            100,
            [
              assistantItem("assistant-1", "Mock", 200),
              toolCallItem("tool-1", 300),
            ],
            "streaming",
          ),
        ]}
        userName="Eric"
        isResponding
      />,
    );
    expect(screen.getByLabelText(/正在运行|is working/)).toBeInTheDocument();

    // Clears once the turn settles and the agent is no longer responding.
    view.rerender(
      <MessageList
        turns={[
          turn(
            "turn-1",
            "hello",
            100,
            [assistantItem("assistant-1", "Mock", 200)],
            "completed",
          ),
        ]}
        userName="Eric"
        isResponding={false}
      />,
    );
    expect(
      screen.queryByLabelText(/正在运行|is working/),
    ).not.toBeInTheDocument();
  });

  it("renders streamed assistant text as markdown while keeping the thread responsive", () => {
    renderWithI18n(
      <MessageList
        turns={[
          turn(
            "turn-1",
            "hello",
            100,
            [
              assistantItem(
                "assistant-1",
                "# Live heading\n\nStill streaming.",
                200,
              ),
            ],
            "streaming",
          ),
        ]}
        userName="Eric"
        isResponding
      />,
    );

    expect(
      screen.getByRole("heading", { level: 1, name: "Live heading" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Still streaming.")).toBeInTheDocument();
  });

  it("flushes a previous text item's literal marker while the turn continues with a tool", () => {
    renderWithI18n(
      <MessageList
        turns={[
          turn(
            "turn-1",
            "hello",
            100,
            [
              assistantItem("assistant-1", "Use literal *", 200),
              toolCallItem("tool-1", 300),
            ],
            "streaming",
          ),
        ]}
        userName="Eric"
        isResponding
      />,
    );

    expect(screen.getByText("Use literal *")).toBeInTheDocument();
  });

  it("keeps following when deferred content changes the rendered height", () => {
    let resizeCallback: ResizeObserverCallback | undefined;
    class TestResizeObserver implements ResizeObserver {
      constructor(callback: ResizeObserverCallback) {
        resizeCallback = callback;
      }

      /** The observed target is irrelevant to this layout-focused regression test. */
      observe() {}

      /** The component only observes one content root for its full lifetime. */
      unobserve() {}

      /** Testing Library owns unmount cleanup after the assertion. */
      disconnect() {}

      /** This test drives the callback directly, so no queued records exist. */
      takeRecords() {
        return [];
      }
    }
    vi.stubGlobal("ResizeObserver", TestResizeObserver);

    const view = renderWithI18n(
      <MessageList
        turns={[
          turn(
            "turn-1",
            "hello",
            100,
            [assistantItem("assistant-1", "Mock", 200)],
            "streaming",
          ),
        ]}
        userName="Eric"
        isResponding
      />,
    );
    const list = screen.getByTestId("message-list");
    Object.defineProperty(list, "scrollHeight", {
      configurable: true,
      value: 480,
    });
    list.scrollTop = 0;

    act(() => resizeCallback?.([], {} as ResizeObserver));

    expect(list.scrollTop).toBe(480);

    Object.defineProperty(list, "scrollHeight", {
      configurable: true,
      value: 560,
    });
    view.rerender(
      <MessageList
        turns={[
          turn(
            "turn-1",
            "hello",
            100,
            [assistantItem("assistant-1", "Mock final code", 200)],
            "completed",
          ),
        ]}
        userName="Eric"
        isResponding={false}
      />,
    );
    act(() => resizeCallback?.([], {} as ResizeObserver));

    expect(list.style.scrollBehavior).toBe("auto");
    expect(list.scrollTop).toBe(560);

    // A programmatic scroll event can arrive after another fast content growth.
    // It must not look like a reader abandoning the live tail.
    Object.defineProperty(list, "clientHeight", {
      configurable: true,
      value: 100,
    });
    Object.defineProperty(list, "scrollHeight", {
      configurable: true,
      value: 720,
    });
    list.scrollTop = 460;
    fireEvent.scroll(list);
    act(() => resizeCallback?.([], {} as ResizeObserver));

    expect(list.scrollTop).toBe(720);

    Object.defineProperty(list, "scrollHeight", {
      configurable: true,
      value: 800,
    });
    list.scrollTop = 0;
    fireEvent.wheel(list, { deltaY: -120 });
    fireEvent.scroll(list);
    act(() => resizeCallback?.([], {} as ResizeObserver));

    expect(list.scrollTop).toBe(0);
  });

  it("stops chasing the tail once the reader scrolls up mid-stream", () => {
    const view = renderWithI18n(
      <MessageList
        turns={[
          turn(
            "turn-1",
            "hello",
            100,
            [assistantItem("assistant-1", "Mock", 200)],
            "streaming",
          ),
        ]}
        userName="Eric"
        isResponding
      />,
    );
    const list = screen.getByTestId("message-list");
    Object.defineProperty(list, "scrollHeight", {
      configurable: true,
      value: 240,
    });
    Object.defineProperty(list, "clientHeight", {
      configurable: true,
      value: 100,
    });

    // An upward wheel gesture is the user intent; scroll events alone can also
    // come from the component's own tail correction.
    fireEvent.wheel(list, { deltaY: -120 });
    list.scrollTop = 0;
    fireEvent.scroll(list);

    view.rerender(
      <MessageList
        turns={[
          turn(
            "turn-1",
            "hello",
            100,
            [assistantItem("assistant-1", "Mock response", 200)],
            "streaming",
          ),
        ]}
        userName="Eric"
        isResponding
      />,
    );

    expect(list.scrollTop).toBe(0);
  });

  it("re-pins to the newest message when the user sends while scrolled up", () => {
    const first = turn("turn-1", "hello", 100, [
      assistantItem("assistant-1", "Mock response", 200),
    ]);
    const view = renderWithI18n(
      <MessageList turns={[first]} userName="Eric" isResponding={false} />,
    );
    const list = screen.getByTestId("message-list");
    Object.defineProperty(list, "scrollHeight", {
      configurable: true,
      value: 240,
    });
    Object.defineProperty(list, "clientHeight", {
      configurable: true,
      value: 100,
    });
    fireEvent.wheel(list, { deltaY: -120 });
    list.scrollTop = 0;
    fireEvent.scroll(list);

    view.rerender(
      <MessageList
        turns={[first, turn("turn-2", "Follow-up", 300, [], "streaming")]}
        userName="Eric"
        isResponding={false}
      />,
    );

    expect(list.scrollTop).toBe(240);
  });
});

describe("ConversationNavigator", () => {
  const turns = [
    turn("turn-1", "**First** question", 100, [
      assistantItem(
        "assistant-1",
        "```markdown\n# First answer\n\nWith `code` and [docs](https://example.com)\n```",
        200,
      ),
    ]),
    turn("turn-2", "Second question", 300, [
      assistantItem("assistant-2", "Second answer", 400),
    ]),
    turn("turn-3", "Third question", 500, [
      assistantItem("assistant-3", "Third answer", 600),
    ]),
  ];

  /** Keeps navigation state local so repeated clicks exercise the real hover-to-boundary transition. */
  function StatefulNavigator() {
    const [activeAnchorId, setActiveAnchorId] = useState("turn-2:user");
    return (
      <ConversationNavigator
        turns={turns}
        activeAnchorId={activeAnchorId}
        isAtTail
        onNavigate={setActiveAnchorId}
        onNavigateToTail={() => {}}
      />
    );
  }

  it("moves one anchor at a time and keeps disabled boundary controls visible", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    const view = renderWithI18n(
      <TooltipProvider>
        <ConversationNavigator
          turns={turns}
          activeAnchorId="turn-2:user"
          isAtTail
          onNavigate={onNavigate}
          onNavigateToTail={() => {}}
        />
      </TooltipProvider>,
    );

    const previousButton = screen.getByRole("button", {
      name: /上一条消息|Previous message/,
    });
    const nextButton = screen.getByRole("button", {
      name: /下一条消息|Next message/,
    });
    await user.click(previousButton);
    await user.click(nextButton);

    expect(onNavigate.mock.calls).toEqual([
      ["turn-1:response"],
      ["turn-2:response"],
    ]);

    view.rerender(
      <TooltipProvider>
        <ConversationNavigator
          turns={turns}
          activeAnchorId="turn-1:user"
          isAtTail
          onNavigate={onNavigate}
          onNavigateToTail={() => {}}
        />
      </TooltipProvider>,
    );
    expect(previousButton).toBeDisabled();
    expect(previousButton).toBeVisible();
    expect(previousButton).toHaveAccessibleName(
      /这是第一条消息|This is the first message/,
    );
    expect(nextButton).toBeEnabled();
    await user.hover(previousButton.parentElement!);
    expect(
      await screen.findByText(/这是第一条消息|This is the first message/),
    ).toBeVisible();

    view.rerender(
      <TooltipProvider>
        <ConversationNavigator
          turns={turns}
          activeAnchorId="turn-3:response"
          isAtTail
          onNavigate={onNavigate}
          onNavigateToTail={() => {}}
        />
      </TooltipProvider>,
    );
    expect(previousButton).toBeEnabled();
    expect(nextButton).toBeDisabled();
    expect(nextButton).toBeVisible();
    expect(nextButton).toHaveAccessibleName(
      /已到达对话底部|You're at the bottom of the conversation/,
    );
    await user.hover(nextButton.parentElement!);
    expect(
      await screen.findByTestId("conversation-navigation-end-hint"),
    ).toHaveTextContent(
      /已到达对话底部|You're at the bottom of the conversation/,
    );
  });

  it("opens the boundary hint when repeated clicks disable the button under the pointer", async () => {
    const user = userEvent.setup();
    renderWithI18n(
      <TooltipProvider>
        <StatefulNavigator />
      </TooltipProvider>,
    );

    const previousButton = screen.getByRole("button", {
      name: /上一条消息|Previous message/,
    });
    await user.click(previousButton);
    await user.click(previousButton);
    expect(previousButton).toBeDisabled();
    expect(
      await screen.findByText(/这是第一条消息|This is the first message/),
    ).toBeVisible();

    const nextButton = screen.getByRole("button", {
      name: /下一条消息|Next message/,
    });
    for (let index = 0; index < 5; index += 1) await user.click(nextButton);
    expect(nextButton).toBeDisabled();
    expect(
      await screen.findByTestId("conversation-navigation-end-hint"),
    ).toHaveTextContent(
      /已到达对话底部|You're at the bottom of the conversation/,
    );
  });

  it("uses the final downward action to reach the thread tail", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    const onNavigateToTail = vi.fn();
    const view = renderWithI18n(
      <TooltipProvider>
        <ConversationNavigator
          turns={turns}
          activeAnchorId="turn-3:response"
          isAtTail={false}
          onNavigate={onNavigate}
          onNavigateToTail={onNavigateToTail}
        />
      </TooltipProvider>,
    );

    const nextButton = screen.getByRole("button", {
      name: /滚动到底部|Scroll to bottom/,
    });
    expect(nextButton).toBeEnabled();
    await user.hover(nextButton.parentElement!);
    expect(
      await screen.findByTestId("conversation-navigation-end-hint"),
    ).toHaveTextContent(/滚动到底部|Scroll to bottom/);
    await user.click(nextButton);
    expect(onNavigate).not.toHaveBeenCalled();
    expect(onNavigateToTail).toHaveBeenCalledOnce();
    expect(
      screen.getByTestId("conversation-navigation-end-hint"),
    ).toHaveTextContent(/滚动到底部|Scroll to bottom/);

    view.rerender(
      <TooltipProvider>
        <ConversationNavigator
          turns={turns}
          activeAnchorId="turn-3:response"
          isAtTail
          onNavigate={onNavigate}
          onNavigateToTail={onNavigateToTail}
        />
      </TooltipProvider>,
    );
    expect(nextButton).toBeDisabled();
    expect(nextButton).toHaveAccessibleName(
      /已到达对话底部|You're at the bottom of the conversation/,
    );
    expect(
      screen.getByTestId("conversation-navigation-end-hint"),
    ).toHaveTextContent(
      /已到达对话底部|You're at the bottom of the conversation/,
    );
    expect(screen.getByTestId("conversation-navigation-end-hint")).toHaveClass(
      "animate-in",
      "fade-in-0",
      "duration-300",
    );
  });

  it("shows no question heading in previews and labels responses as Ora", () => {
    renderWithI18n(
      <ConversationNavigator
        turns={turns}
        activeAnchorId="turn-2:user"
        isAtTail
        onNavigate={() => {}}
        onNavigateToTail={() => {}}
      />,
    );

    fireEvent.mouseEnter(
      screen.getByRole("button", { name: /问题 1|Question 1/ }),
      { clientY: 10 },
    );
    const questionPreview = screen.getByTestId("conversation-anchor-preview");
    expect(questionPreview).toHaveTextContent("First question");
    expect(questionPreview.querySelector("strong")).toHaveTextContent("First");
    expect(questionPreview).not.toHaveTextContent(/问题 1|Question 1/);

    fireEvent.mouseEnter(
      screen.getByRole("button", { name: /回复 1|Response 1/ }),
      { clientY: 10 },
    );
    const responsePreview = screen.getByTestId("conversation-anchor-preview");
    expect(responsePreview).toHaveTextContent(
      "OraFirst answer With code and docs",
    );
    expect(responsePreview.querySelector("p.font-semibold")).toHaveTextContent(
      "First answer",
    );
    expect(responsePreview.querySelector("code")).toHaveTextContent("code");
    expect(responsePreview.querySelector("a")).toBeNull();
    expect(responsePreview.querySelector("pre")).toBeNull();
    expect(responsePreview).not.toHaveTextContent(/回复 1|Response 1/);
  });

  it("parses complete Markdown before visually clipping long previews", () => {
    const longMarkdown = `${"prefix ".repeat(20)}**complete marker**`;
    const longTurns = [
      turn("long-1", "Question", 100, [
        assistantItem("long-answer", longMarkdown, 200),
      ]),
      turns[1],
      turns[2],
    ];
    renderWithI18n(
      <ConversationNavigator
        turns={longTurns}
        activeAnchorId="long-1:response"
        isAtTail
        onNavigate={() => {}}
        onNavigateToTail={() => {}}
      />,
    );

    fireEvent.mouseEnter(
      screen.getByRole("button", { name: /回复 1|Response 1/ }),
      { clientY: 10 },
    );
    expect(
      screen.getByTestId("conversation-anchor-preview").querySelector("strong"),
    ).toHaveTextContent("complete marker");
  });

  it("renders fenced code as an unframed compact excerpt", () => {
    const codeTurns = [
      turn("code-1", "Question", 100, [
        assistantItem(
          "code-answer",
          "```python\n# Python example\ndef fibonacci(n):\n    return n\n```",
          200,
        ),
      ]),
      turns[1],
      turns[2],
    ];
    renderWithI18n(
      <ConversationNavigator
        turns={codeTurns}
        activeAnchorId="code-1:response"
        isAtTail
        onNavigate={() => {}}
        onNavigateToTail={() => {}}
      />,
    );

    fireEvent.mouseEnter(
      screen.getByRole("button", { name: /回复 1|Response 1/ }),
      { clientY: 10 },
    );
    const codeBlock = screen
      .getByTestId("conversation-anchor-preview")
      .querySelector("[data-preview-code-block]");
    expect(codeBlock).toHaveTextContent(
      "# Python example def fibonacci(n): return n",
    );
    expect(codeBlock).toHaveClass("border-l-2", "pl-2");
    expect(codeBlock).not.toHaveClass("bg-muted", "rounded-sm");
  });
});
