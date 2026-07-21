import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ChatTurn } from "@ora/chat";
import { AppI18nProvider } from "../../i18n/i18n";
import { ChatView } from "./chat-view";
import { Composer } from "./composer";
import { MessageList } from "./message-list";
import type { ChatSuggestion } from "../../lib/types";

const TEST_SUGGESTIONS: readonly ChatSuggestion[] = [
  { id: "chat", text: { "zh-CN": "解释 greeting 的工作方式", "en-US": "Explain greeting" } },
  { id: "success", text: { "zh-CN": "实现并验证 greeting", "en-US": "Implement greeting" } },
  { id: "failure", text: { "zh-CN": "修改不存在的文件", "en-US": "Modify missing file" } },
];

/** Renders chat components with the same isolated i18n provider as AppShell. */
function renderWithI18n(element: React.ReactNode) {
  return render(<AppI18nProvider>{element}</AppI18nProvider>);
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

  it("shows a stop action while the response turn is active", async () => {
    const user = userEvent.setup();
    const onCancel = vi.fn();
    renderWithI18n(<Composer onSend={() => {}} onCancel={onCancel} isResponding />);

    await user.click(screen.getByRole("button", { name: /停止生成|Stop generating/ }));

    expect(onCancel).toHaveBeenCalledOnce();
    expect(screen.queryByRole("button", { name: /发送消息|Send message/ })).not.toBeInTheDocument();
  });
});

describe("ChatView", () => {
  it("offers the temporary ACP mock scenarios in a new conversation", async () => {
    const user = userEvent.setup();
    const onSend = vi.fn();
    renderWithI18n(
      <ChatView
        conversation={undefined}
        userName="Eric"
        isResponding={false}
        error={null}
        suggestions={TEST_SUGGESTIONS}
        onSend={onSend}
        onCancel={() => {}}
      />,
    );

    expect(screen.getByRole("button", { name: "解释 greeting 的工作方式" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "实现并验证 greeting" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "修改不存在的文件" }));

    expect(onSend).toHaveBeenCalledWith("修改不存在的文件");
  });

  it("disables composition and shows the unavailable Agent session error", () => {
    renderWithI18n(
      <ChatView
        conversation={undefined}
        userName="Eric"
        isResponding={false}
        error="Agent session unavailable"
        disabled
        onSend={() => {}}
        onCancel={() => {}}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent("Agent session unavailable");
    expect(screen.getByRole("textbox")).toBeDisabled();
  });

  it("keeps the disabled hint shut when the pointer never left the enabled composer", async () => {
    const user = userEvent.setup();
    const view = renderWithI18n(
      <ChatView conversation={undefined} userName="Eric" isResponding={false} error={null} onSend={() => {}} />,
    );

    // Hover the composer while it has no hint. The real app then slides the
    // composer out from under the pointer, so no pointerleave ever arrives.
    await user.hover(screen.getByRole("textbox"));

    view.rerender(
      <AppI18nProvider>
        <ChatView
          conversation={undefined}
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

  it("slides the same composer node down when the first turn arrives", () => {
    // jsdom has no layout and no Web Animations API, so both are stood up here:
    // the rects drive the FLIP delta and the spy captures the resulting keyframes.
    let top = 300;
    const rectSpy = vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(() => ({ top }) as DOMRect);
    const animate = vi.fn();
    Object.defineProperty(Element.prototype, "animate", { configurable: true, writable: true, value: animate });

    const view = renderWithI18n(
      <ChatView conversation={undefined} userName="Eric" isResponding={false} error={null} onSend={() => {}} onCancel={() => {}} />,
    );
    const landingComposer = screen.getByRole("textbox");
    top = 800;
    view.rerender(
      <AppI18nProvider>
        <ChatView conversation={{ turns: [createTurn()], error: null }} userName="Eric" isResponding={false} error={null} onSend={() => {}} onCancel={() => {}} />
      </AppI18nProvider>,
    );

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
  it("replaces the typing indicator once a thought chunk arrives", () => {
    const streaming = createTurn({ status: "streaming", items: [] });
    const view = renderWithI18n(<MessageList turns={[streaming]} userName="Eric" isResponding />);
    expect(screen.getByLabelText(/正在输入|is typing/)).toBeInTheDocument();

    view.rerender(
      <AppI18nProvider>
        <MessageList
          turns={[createTurn({
            status: "streaming",
            items: [{ kind: "thought", id: "thought-1", content: "Inspecting", createdAt: 200 }],
          })]}
          userName="Eric"
          isResponding
        />
      </AppI18nProvider>,
    );

    expect(screen.queryByLabelText(/正在输入|is typing/)).not.toBeInTheDocument();
    expect(screen.getByText("Inspecting")).toBeInTheDocument();
  });

  it("renders plan, tool lifecycle, unified diff, and raw protocol data", async () => {
    const user = userEvent.setup();
    const turn = createTurn({
      items: [
        {
          kind: "plan",
          id: "plan-1",
          entries: [{ content: "Update app", priority: "high", status: "completed" }],
          createdAt: 200,
          updatedAt: 300,
        },
        {
          kind: "toolCall",
          id: "tool-1",
          title: "Update the greeting implementation",
          toolKind: "edit",
          status: "completed",
          locations: [{ path: "/workspace/ora/src/app.ts", line: 1 }],
          content: [{
            type: "diff",
            path: "/workspace/ora/src/app.ts",
            oldText: "return name;\n",
            newText: "return name.trim();\n",
          }],
          rawInput: { path: "/workspace/ora/src/app.ts" },
          rawOutput: { changed: true },
          createdAt: 200,
          updatedAt: 300,
        },
      ],
    });
    renderWithI18n(<MessageList turns={[turn]} userName="Eric" isResponding={false} />);

    await user.click(screen.getByText("Update the greeting implementation"));

    expect(screen.getAllByTitle("/workspace/ora/src/app.ts")).toHaveLength(2);
    expect(screen.getByText("return name.trim();")).toBeInTheDocument();
    expect(screen.getByText(/原始数据|Raw data/)).toBeInTheDocument();
  });

  it("groups adjacent exploration tools into one expandable activity summary", async () => {
    const user = userEvent.setup();
    const readTool = (id: string, path: string) => ({
      kind: "toolCall" as const,
      id,
      title: `Read ${path}`,
      toolKind: "read" as const,
      status: "completed" as const,
      locations: [{ path }],
      content: [],
      createdAt: 200,
      updatedAt: 300,
    });
    const turn = createTurn({
      items: [
        readTool("read-1", "/workspace/ora/src/app.ts"),
        readTool("read-2", "/workspace/ora/src/store.ts"),
        { ...readTool("search-1", "/workspace/ora/src/types.ts"), title: "Search types", toolKind: "search" as const },
        readTool("read-4", "/workspace/ora/src/client.ts"),
      ],
    });
    renderWithI18n(<MessageList turns={[turn]} userName="Eric" isResponding={false} />);

    const summary = screen.getByRole("button", { name: /已分析 4 项资源|Analyzed 4 resources/ });
    expect(summary).toHaveTextContent("app.ts, store.ts, types.ts +1");
    expect(screen.queryByText("Read /workspace/ora/src/app.ts")).not.toBeInTheDocument();

    await user.click(summary);

    expect(screen.getByText("Read /workspace/ora/src/app.ts")).toBeInTheDocument();
    expect(screen.getByText("Read /workspace/ora/src/client.ts")).toBeInTheDocument();
  });

  it("keeps a non-read tool between separate read activities", () => {
    const turn = createTurn({
      items: [
        { kind: "toolCall", id: "read-1", title: "Read app", toolKind: "read", status: "completed", locations: [], content: [], createdAt: 200, updatedAt: 300 },
        { kind: "toolCall", id: "edit-1", title: "Edit app", toolKind: "edit", status: "completed", locations: [], content: [], createdAt: 200, updatedAt: 300 },
        { kind: "toolCall", id: "read-2", title: "Read test", toolKind: "read", status: "completed", locations: [], content: [], createdAt: 200, updatedAt: 300 },
      ],
    });
    renderWithI18n(<MessageList turns={[turn]} userName="Eric" isResponding={false} />);

    expect(screen.queryByText(/已分析 2 项资源|Analyzed 2 resources/)).not.toBeInTheDocument();
    expect(screen.getByText("Read app")).toBeInTheDocument();
    expect(screen.getByText("Edit app")).toBeInTheDocument();
    expect(screen.getByText("Read test")).toBeInTheDocument();
  });

  it("counts unique resources instead of repeated tool calls", () => {
    const path = "/workspace/ora/src/app.ts";
    const turn = createTurn({
      items: [
        { kind: "toolCall", id: "read-1", title: "Read app", toolKind: "read", status: "completed", locations: [{ path }], content: [], createdAt: 200, updatedAt: 300 },
        { kind: "toolCall", id: "read-2", title: "Read app again", toolKind: "read", status: "completed", locations: [{ path }], content: [], createdAt: 200, updatedAt: 300 },
      ],
    });
    renderWithI18n(<MessageList turns={[turn]} userName="Eric" isResponding={false} />);

    expect(screen.getByRole("button", { name: /已分析 1 项资源|Analyzed 1 resource/ })).toBeInTheDocument();
  });

  it("summarizes adjacent file changes with aggregate diff counts", async () => {
    const user = userEvent.setup();
    const editTool = (id: string, path: string, oldText: string, newText: string) => ({
      kind: "toolCall" as const,
      id,
      title: `Edit ${path}`,
      toolKind: "edit" as const,
      status: "completed" as const,
      locations: [{ path }],
      content: [{ type: "diff" as const, path, oldText, newText }],
      createdAt: 200,
      updatedAt: 300,
    });
    const turn = createTurn({
      items: [
        editTool("edit-1", "/workspace/ora/src/app.ts", "return name;\n", "return name.trim();\n"),
        editTool("edit-2", "/workspace/ora/src/store.ts", "const ready = false;\n", "const ready = true;\n"),
      ],
    });
    renderWithI18n(<MessageList turns={[turn]} userName="Eric" isResponding={false} />);

    const summary = screen.getByRole("button", { name: /已修改 2 个文件|Changed 2 files/ });
    expect(summary).toHaveTextContent("app.ts, store.ts");
    expect(summary).toHaveTextContent("+2");
    expect(summary).toHaveTextContent("-2");

    await user.click(summary);

    expect(screen.getByText("Edit /workspace/ora/src/app.ts")).toBeInTheDocument();
    expect(screen.getByText("Edit /workspace/ora/src/store.ts")).toBeInTheDocument();
  });

  it("keeps a failed command group expanded", () => {
    const turn = createTurn({
      items: [
        { kind: "toolCall", id: "command-1", title: "Run typecheck", toolKind: "execute", status: "completed", locations: [], content: [], rawOutput: { exitCode: 0 }, createdAt: 200, updatedAt: 300 },
        { kind: "toolCall", id: "command-2", title: "Run tests", toolKind: "execute", status: "failed", locations: [], content: [], rawOutput: { exitCode: 1 }, createdAt: 200, updatedAt: 300 },
      ],
    });
    renderWithI18n(<MessageList turns={[turn]} userName="Eric" isResponding={false} />);

    expect(screen.getByRole("button", { name: /2 条命令执行失败|Command batch failed \(2 commands\)/ })).toBeInTheDocument();
    expect(screen.getByText("Run typecheck")).toBeInTheDocument();
    expect(screen.getByText("Run tests")).toBeInTheDocument();
  });

  it("stops chasing the tail once the reader scrolls up mid-stream", () => {
    const view = renderWithI18n(<MessageList turns={[createTurn({ status: "streaming" })]} userName="Eric" isResponding />);
    const list = screen.getByTestId("message-list");
    Object.defineProperty(list, "scrollHeight", { configurable: true, value: 240 });
    Object.defineProperty(list, "clientHeight", { configurable: true, value: 100 });
    list.scrollTop = 0;
    fireEvent.scroll(list);

    view.rerender(
      <AppI18nProvider>
        <MessageList
          turns={[createTurn({
            status: "streaming",
            items: [{ kind: "message", id: "message-1", role: "assistant", content: "Growing", createdAt: 200 }],
          })]}
          userName="Eric"
          isResponding
        />
      </AppI18nProvider>,
    );

    expect(list.scrollTop).toBe(0);
  });
});

/** Builds one representative response turn for component tests. */
function createTurn(overrides: Partial<ChatTurn> = {}): ChatTurn {
  return {
    id: "turn-1",
    userMessage: {
      kind: "message",
      id: "user-1",
      role: "user",
      content: "hello",
      createdAt: 100,
    },
    items: [{ kind: "message", id: "assistant-1", role: "assistant", content: "Done", createdAt: 200 }],
    status: "completed",
    stopReason: "end_turn",
    error: null,
    createdAt: 100,
    ...overrides,
  };
}
