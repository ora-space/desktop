import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ChatTurn } from "@ora/chat";
import { AppI18nProvider } from "../../i18n/i18n";
import { ChatView } from "./chat-view";
import { Composer } from "./composer";
import { MessageList } from "./message-list";

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
        onSend={onSend}
        onCancel={() => {}}
      />,
    );

    expect(screen.getByRole("button", { name: "解释 greeting 的工作方式" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "实现 greeting" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "实现并测试 greeting" })).toBeInTheDocument();
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
