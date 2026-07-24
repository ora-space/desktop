import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import type { ChatMessage, ChatThought, ChatToolCall, ChatTurn, ChatTurnItem } from "@ora/chat";
import { TooltipProvider } from "@ora/ui";
import { AppI18nProvider } from "../../i18n/i18n";
import { ChatView } from "./chat-view";
import { Composer } from "./composer";
import { ConversationNavigator } from "./conversation-navigator";
import { MessageList } from "./message-list";

/** Renders chat components with the same isolated i18n provider as AppShell. */
function renderWithI18n(element: React.ReactNode) {
  return render(<AppI18nProvider>{element}</AppI18nProvider>);
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
    userMessage: { kind: "message", id: `${id}-user`, role: "user", content, createdAt },
    items,
    status,
    stopReason: null,
    error: null,
    createdAt,
  };
}

/** Builds one assistant text item that lives inside a response turn. */
function assistantItem(id: string, content: string, createdAt: number): ChatMessage {
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
function completedReadItem(id: string, path: string, createdAt: number): ChatToolCall {
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
function activeReadItem(id: string, path: string, createdAt: number): ChatToolCall {
  return {
    ...completedReadItem(id, path, createdAt),
    status: "in_progress",
  };
}

/** Builds one completed edit with a path for change-group coverage. */
function completedEditItem(id: string, path: string, createdAt: number): ChatToolCall {
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
function completedCommandItem(id: string, title: string, createdAt: number): ChatToolCall {
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
function thoughtItem(id: string, content: string, createdAt: number): ChatThought {
  return { kind: "thought", id, content, createdAt };
}

describe("Composer", () => {
  it("sends trimmed text with Enter and clears the textarea", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "  hello{Enter}");

    expect(onSend).toHaveBeenCalledWith("hello");
    expect(textarea).toHaveValue("");
  });

  it("uses Shift+Enter for a newline without sending", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    renderWithI18n(<Composer onSend={onSend} isResponding={false} />);

    const textarea = screen.getByRole("textbox");
    await user.type(textarea, "first{Shift>}{Enter}{/Shift}second");

    expect(onSend).not.toHaveBeenCalled();
    expect(textarea).toHaveValue("first\nsecond");
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

    expect(screen.getByRole("alert")).toHaveTextContent("Agent session unavailable");
    expect(screen.getByRole("textbox")).toBeDisabled();
    expect(screen.getAllByRole("button")).toEqual(
      expect.arrayContaining([expect.objectContaining({ disabled: true })]),
    );
  });

  it("keeps the disabled hint shut when the pointer never left the enabled composer", async () => {
    const user = userEvent.setup();
    const view = renderWithI18n(
      <ChatView turns={[]} userName="Eric" isResponding={false} error={null} onSend={() => {}} />,
    );

    // Hover the composer while it has no hint. The real app then slides the
    // composer out from under the pointer, so no pointerleave ever arrives.
    await user.hover(screen.getByRole("textbox"));

    view.rerender(
      <AppI18nProvider>
        <ChatView
          turns={[]}
          userName="Eric"
          isResponding={false}
          error={null}
          disabled
          disabledHint="pick a project"
          onSend={() => {}}
        />
      </AppI18nProvider>,
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

    const composer = screen.getByRole("textbox").closest('[data-slot="composer"]');
    const context = screen.getByText("Ora / frontend").closest('[data-slot="composer-context"]');
    expect(composer).not.toBeNull();
    expect(context).not.toBeNull();
    expect(composer?.contains(context)).toBe(false);
    expect(context?.nextElementSibling?.querySelector('[data-slot="composer"]')).toBe(composer);
  });

  it("shows the history loading indicator without the landing copy while a session loads", () => {
    renderWithI18n(
      <ChatView turns={[]} userName="Eric" isResponding={false} isLoading error={null} onSend={() => {}} />,
    );

    // Thread layout: the loading status stands in for the yet-to-arrive turns and
    // the landing heading/suggestions are gone, so the composer has slid down.
    expect(screen.getByRole("status", { name: /加载历史|Loading history/ })).toBeInTheDocument();
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
      <ChatView turns={[]} userName="Eric" isResponding={false} error={null} onSend={() => {}} />,
    );

    // Selecting a session flips it into the loading thread layout: the composer
    // slides down here, before any turn exists.
    top = 800;
    view.rerender(
      <AppI18nProvider>
        <ChatView turns={[]} userName="Eric" isResponding={false} isLoading error={null} onSend={() => {}} />
      </AppI18nProvider>,
    );
    expect(animate).toHaveBeenCalledTimes(1);

    // History arriving is not a landing→thread transition, so it must not replay
    // the slide — otherwise the composer animates twice for one selection.
    view.rerender(
      <AppI18nProvider>
        <ChatView
          turns={[turn("turn-1", "hello", 100)]}
          userName="Eric"
          isResponding={false}
          error={null}
          onSend={() => {}}
        />
      </AppI18nProvider>,
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
      <ChatView turns={[]} userName="Eric" isResponding={false} error={null} onSend={() => {}} />,
    );
    const landingComposer = screen.getByRole("textbox");

    top = 800;
    view.rerender(
      <AppI18nProvider>
        <ChatView
          turns={[turn("turn-1", "hello", 100)]}
          userName="Eric"
          isResponding={false}
          error={null}
          onSend={() => {}}
        />
      </AppI18nProvider>,
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

    await user.click(screen.getByRole("button", { name: /文件读取完成|File reading complete/ }));
    const readBatch = screen.getByRole("button", { name: /读取 4 个文件|Read 4 files/ });
    expect(screen.queryByRole("button", { name: /读取\s*a\.md|Read\s*a\.md/ })).toBeNull();

    await user.click(readBatch);
    expect(screen.getByRole("button", { name: /读取\s*a\.md|Read\s*a\.md/ })).toBeVisible();
    expect(screen.getByRole("button", { name: /读取\s*d\.md|Read\s*d\.md/ })).toBeVisible();
  });

  it("keeps reads, edits, and commands as distinct perceptible activity groups", () => {
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

    expect(screen.getByRole("button", { name: /文件读取完成|File reading complete/ })).toBeVisible();
    expect(screen.getByRole("button", { name: /已修改 2 个文件|Changed 2 files/ })).toBeVisible();
    expect(screen.getByRole("button", { name: /已执行 2 条命令|Ran 2 commands/ })).toBeVisible();
  });

  it("surfaces a domain-neutral current file while exploration is streaming", () => {
    renderWithI18n(
      <MessageList
        turns={[
          turn("turn-1", "Inspect the entry point", 100, [
            thoughtItem("thought-1", "Locating the application entry point", 200),
            activeReadItem("read-1", "reports/q2.pdf", 300),
          ], "streaming"),
        ]}
        userName="Eric"
        isResponding
      />,
    );

    const activity = screen.getByRole("button", { name: /正在读取 q2\.pdf|Reading q2\.pdf/ });
    expect(activity).toHaveTextContent(/1 个文件 · 1 次分析|1 file · 1 analysis step/);
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

    const activity = screen.getByRole("button", { name: /文件读取完成|File reading complete/ });
    expect(activity).toHaveTextContent(/2 个文件 · 2 次分析|2 files · 2 analysis steps/);
    expect(screen.queryByText("Checking project configuration")).toBeNull();

    await user.click(activity);
    expect(screen.getByText("Cargo.toml")).toBeVisible();
    expect(screen.getByText("main.rs")).toBeVisible();
    const firstThought = screen.getByRole("button", { name: /Checking project configuration/ });
    const secondThought = screen.getByRole("button", { name: /Finding the relevant source/ });
    await user.click(firstThought);
    expect(screen.getAllByText("Checking project configuration")).toHaveLength(2);

    await user.click(secondThought);
    expect(screen.getAllByText("Checking project configuration")).toHaveLength(1);
    expect(screen.getAllByText("Finding the relevant source")).toHaveLength(2);
  });

  it("shows the running indicator while working but hides it as the answer streams", () => {
    const view = renderWithI18n(
      <MessageList turns={[turn("turn-1", "hello", 100, [], "streaming")]} userName="Eric" isResponding />,
    );
    // Waiting for the first output: the indicator stands in for the empty turn.
    expect(screen.getByLabelText(/正在运行|is working/)).toBeInTheDocument();

    // Answer body streaming in: the growing text is signal enough, so it hides.
    view.rerender(
      <AppI18nProvider>
        <MessageList
          turns={[turn("turn-1", "hello", 100, [assistantItem("assistant-1", "Mock", 200)], "streaming")]}
          userName="Eric"
          isResponding
        />
      </AppI18nProvider>,
    );
    expect(screen.queryByLabelText(/正在运行|is working/)).not.toBeInTheDocument();

    // Back to working — a tool call trails the text — so the indicator returns.
    view.rerender(
      <AppI18nProvider>
        <MessageList
          turns={[turn("turn-1", "hello", 100, [assistantItem("assistant-1", "Mock", 200), toolCallItem("tool-1", 300)], "streaming")]}
          userName="Eric"
          isResponding
        />
      </AppI18nProvider>,
    );
    expect(screen.getByLabelText(/正在运行|is working/)).toBeInTheDocument();

    // Clears once the turn settles and the agent is no longer responding.
    view.rerender(
      <AppI18nProvider>
        <MessageList
          turns={[turn("turn-1", "hello", 100, [assistantItem("assistant-1", "Mock", 200)], "completed")]}
          userName="Eric"
          isResponding={false}
        />
      </AppI18nProvider>,
    );
    expect(screen.queryByLabelText(/正在运行|is working/)).not.toBeInTheDocument();
  });

  it("keeps scrolling as streamed content grows within the same message", () => {
    const view = renderWithI18n(
      <MessageList
        turns={[turn("turn-1", "hello", 100, [assistantItem("assistant-1", "Mock", 200)], "streaming")]}
        userName="Eric"
        isResponding
      />,
    );
    const list = screen.getByTestId("message-list");
    Object.defineProperty(list, "scrollHeight", { configurable: true, value: 240 });
    list.scrollTop = 0;

    view.rerender(
      <AppI18nProvider>
        <MessageList
          turns={[turn("turn-1", "hello", 100, [assistantItem("assistant-1", "Mock response", 200)], "streaming")]}
          userName="Eric"
          isResponding
        />
      </AppI18nProvider>,
    );

    expect(list.scrollTop).toBe(240);
  });

  it("stops chasing the tail once the reader scrolls up mid-stream", () => {
    const view = renderWithI18n(
      <MessageList
        turns={[turn("turn-1", "hello", 100, [assistantItem("assistant-1", "Mock", 200)], "streaming")]}
        userName="Eric"
        isResponding
      />,
    );
    const list = screen.getByTestId("message-list");
    Object.defineProperty(list, "scrollHeight", { configurable: true, value: 240 });
    Object.defineProperty(list, "clientHeight", { configurable: true, value: 100 });

    // Scrolling far from the bottom is the signal that the reader is reading
    // history rather than following the stream.
    list.scrollTop = 0;
    fireEvent.scroll(list);

    view.rerender(
      <AppI18nProvider>
        <MessageList
          turns={[turn("turn-1", "hello", 100, [assistantItem("assistant-1", "Mock response", 200)], "streaming")]}
          userName="Eric"
          isResponding
        />
      </AppI18nProvider>,
    );

    expect(list.scrollTop).toBe(0);
  });

  it("re-pins to the newest message when the user sends while scrolled up", () => {
    const first = turn("turn-1", "hello", 100, [assistantItem("assistant-1", "Mock response", 200)]);
    const view = renderWithI18n(
      <MessageList turns={[first]} userName="Eric" isResponding={false} />,
    );
    const list = screen.getByTestId("message-list");
    Object.defineProperty(list, "scrollHeight", { configurable: true, value: 240 });
    Object.defineProperty(list, "clientHeight", { configurable: true, value: 100 });
    list.scrollTop = 0;
    fireEvent.scroll(list);

    view.rerender(
      <AppI18nProvider>
        <MessageList
          turns={[first, turn("turn-2", "Follow-up", 300, [], "streaming")]}
          userName="Eric"
          isResponding={false}
        />
      </AppI18nProvider>,
    );

    expect(list.scrollTop).toBe(240);
  });
});

describe("ConversationNavigator", () => {
  const turns = [
    turn("turn-1", "**First** question", 100, [assistantItem("assistant-1", "```markdown\n# First answer\n\nWith `code` and [docs](https://example.com)\n```", 200)]),
    turn("turn-2", "Second question", 300, [assistantItem("assistant-2", "Second answer", 400)]),
    turn("turn-3", "Third question", 500, [assistantItem("assistant-3", "Third answer", 600)]),
  ];

  /** Keeps navigation state local so repeated clicks exercise the real hover-to-boundary transition. */
  function StatefulNavigator() {
    const [activeAnchorId, setActiveAnchorId] = useState("turn-2:user");
    return <ConversationNavigator turns={turns} activeAnchorId={activeAnchorId} onNavigate={setActiveAnchorId} />;
  }

  it("moves one anchor at a time and keeps disabled boundary controls visible", async () => {
    const user = userEvent.setup();
    const onNavigate = vi.fn();
    const view = renderWithI18n(
      <TooltipProvider>
        <ConversationNavigator turns={turns} activeAnchorId="turn-2:user" onNavigate={onNavigate} />
      </TooltipProvider>,
    );

    const previousButton = screen.getByRole("button", { name: /上一条消息|Previous message/ });
    const nextButton = screen.getByRole("button", { name: /下一条消息|Next message/ });
    await user.click(previousButton);
    await user.click(nextButton);

    expect(onNavigate.mock.calls).toEqual([["turn-1:response"], ["turn-2:response"]]);

    view.rerender(
      <AppI18nProvider>
        <TooltipProvider>
          <ConversationNavigator turns={turns} activeAnchorId="turn-1:user" onNavigate={onNavigate} />
        </TooltipProvider>
      </AppI18nProvider>,
    );
    expect(previousButton).toBeDisabled();
    expect(previousButton).toBeVisible();
    expect(previousButton).toHaveAccessibleName(/这是第一条消息|This is the first message/);
    expect(nextButton).toBeEnabled();
    await user.hover(previousButton.parentElement!);
    expect(await screen.findByText(/这是第一条消息|This is the first message/)).toBeVisible();

    view.rerender(
      <AppI18nProvider>
        <TooltipProvider>
          <ConversationNavigator turns={turns} activeAnchorId="turn-3:response" onNavigate={onNavigate} />
        </TooltipProvider>
      </AppI18nProvider>,
    );
    expect(previousButton).toBeEnabled();
    expect(nextButton).toBeDisabled();
    expect(nextButton).toBeVisible();
    expect(nextButton).toHaveAccessibleName(/这是最后一条消息|This is the last message/);
    await user.hover(nextButton.parentElement!);
    expect(await screen.findByText(/这是最后一条消息|This is the last message/)).toBeVisible();
  });

  it("opens the boundary hint when repeated clicks disable the button under the pointer", async () => {
    const user = userEvent.setup();
    renderWithI18n(
      <TooltipProvider>
        <StatefulNavigator />
      </TooltipProvider>,
    );

    const previousButton = screen.getByRole("button", { name: /上一条消息|Previous message/ });
    await user.click(previousButton);
    await user.click(previousButton);
    expect(previousButton).toBeDisabled();
    expect(await screen.findByText(/这是第一条消息|This is the first message/)).toBeVisible();

    const nextButton = screen.getByRole("button", { name: /下一条消息|Next message/ });
    for (let index = 0; index < 5; index += 1) await user.click(nextButton);
    expect(nextButton).toBeDisabled();
    expect(await screen.findByText(/这是最后一条消息|This is the last message/)).toBeVisible();
  });

  it("shows no question heading in previews and labels responses as Ora", () => {
    renderWithI18n(
      <ConversationNavigator turns={turns} activeAnchorId="turn-2:user" onNavigate={() => {}} />,
    );

    fireEvent.mouseEnter(screen.getByRole("button", { name: /问题 1|Question 1/ }), { clientY: 10 });
    const questionPreview = screen.getByTestId("conversation-anchor-preview");
    expect(questionPreview).toHaveTextContent("First question");
    expect(questionPreview.querySelector("strong")).toHaveTextContent("First");
    expect(questionPreview).not.toHaveTextContent(/问题 1|Question 1/);

    fireEvent.mouseEnter(screen.getByRole("button", { name: /回复 1|Response 1/ }), { clientY: 10 });
    const responsePreview = screen.getByTestId("conversation-anchor-preview");
    expect(responsePreview).toHaveTextContent("OraFirst answer With code and docs");
    expect(responsePreview.querySelector("p.font-semibold")).toHaveTextContent("First answer");
    expect(responsePreview.querySelector("code")).toHaveTextContent("code");
    expect(responsePreview.querySelector("a")).toBeNull();
    expect(responsePreview.querySelector("pre")).toBeNull();
    expect(responsePreview).not.toHaveTextContent(/回复 1|Response 1/);
  });

  it("parses complete Markdown before visually clipping long previews", () => {
    const longMarkdown = `${"prefix ".repeat(20)}**complete marker**`;
    const longTurns = [
      turn("long-1", "Question", 100, [assistantItem("long-answer", longMarkdown, 200)]),
      turns[1],
      turns[2],
    ];
    renderWithI18n(
      <ConversationNavigator turns={longTurns} activeAnchorId="long-1:response" onNavigate={() => {}} />,
    );

    fireEvent.mouseEnter(screen.getByRole("button", { name: /回复 1|Response 1/ }), { clientY: 10 });
    expect(screen.getByTestId("conversation-anchor-preview").querySelector("strong")).toHaveTextContent("complete marker");
  });

  it("renders fenced code as an unframed compact excerpt", () => {
    const codeTurns = [
      turn("code-1", "Question", 100, [assistantItem("code-answer", "```python\n# Python example\ndef fibonacci(n):\n    return n\n```", 200)]),
      turns[1],
      turns[2],
    ];
    renderWithI18n(
      <ConversationNavigator turns={codeTurns} activeAnchorId="code-1:response" onNavigate={() => {}} />,
    );

    fireEvent.mouseEnter(screen.getByRole("button", { name: /回复 1|Response 1/ }), { clientY: 10 });
    const codeBlock = screen.getByTestId("conversation-anchor-preview").querySelector("[data-preview-code-block]");
    expect(codeBlock).toHaveTextContent("# Python example def fibonacci(n): return n");
    expect(codeBlock).toHaveClass("border-l-2", "pl-2");
    expect(codeBlock).not.toHaveClass("bg-muted", "rounded-sm");
  });
});
