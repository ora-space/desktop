import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ChatMessage } from "@ora/chat";
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
});

describe("ChatView", () => {
  it("disables composition and shows the unavailable Agent session error", () => {
    renderWithI18n(
      <ChatView
        messages={[]}
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
      <ChatView messages={[]} userName="Eric" isResponding={false} error={null} onSend={() => {}} />,
    );

    // Hover the composer while it has no hint. The real app then slides the
    // composer out from under the pointer, so no pointerleave ever arrives.
    await user.hover(screen.getByRole("textbox"));

    view.rerender(
      <AppI18nProvider>
        <ChatView
          messages={[]}
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
      <ChatView messages={[]} userName="Eric" isResponding={false} error={null} onSend={() => {}} />,
    );
    const landingComposer = screen.getByRole("textbox");

    top = 800;
    view.rerender(
      <AppI18nProvider>
        <ChatView
          messages={[{ id: "user-1", role: "user", content: "hello", createdAt: 100 }]}
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
  const userMessage: ChatMessage = {
    id: "user-1",
    role: "user",
    content: "hello",
    createdAt: 100,
  };

  it("replaces the typing indicator once the first assistant chunk arrives", () => {
    const view = renderWithI18n(
      <MessageList messages={[userMessage]} userName="Eric" isResponding />,
    );
    expect(screen.getByLabelText(/正在输入|is typing/)).toBeInTheDocument();

    view.rerender(
      <AppI18nProvider>
        <MessageList
          messages={[
            userMessage,
            {
              id: "assistant-1",
              role: "assistant",
              content: "Mock",
              createdAt: 200,
            },
          ]}
          userName="Eric"
          isResponding
        />
      </AppI18nProvider>,
    );

    expect(screen.queryByLabelText(/正在输入|is typing/)).not.toBeInTheDocument();
  });

  it("keeps scrolling as streamed content grows within the same message", () => {
    const assistantMessage: ChatMessage = {
      id: "assistant-1",
      role: "assistant",
      content: "Mock",
      createdAt: 200,
    };
    const view = renderWithI18n(
      <MessageList
        messages={[userMessage, assistantMessage]}
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
          messages={[
            userMessage,
            { ...assistantMessage, content: "Mock response" },
          ]}
          userName="Eric"
          isResponding
        />
      </AppI18nProvider>,
    );

    expect(list.scrollTop).toBe(240);
  });

  it("stops chasing the tail once the reader scrolls up mid-stream", () => {
    const assistantMessage: ChatMessage = {
      id: "assistant-1",
      role: "assistant",
      content: "Mock",
      createdAt: 200,
    };
    const view = renderWithI18n(
      <MessageList
        messages={[userMessage, assistantMessage]}
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
          messages={[
            userMessage,
            { ...assistantMessage, content: "Mock response" },
          ]}
          userName="Eric"
          isResponding
        />
      </AppI18nProvider>,
    );

    expect(list.scrollTop).toBe(0);
  });

  it("re-pins to the newest message when the user sends while scrolled up", () => {
    const assistantMessage: ChatMessage = {
      id: "assistant-1",
      role: "assistant",
      content: "Mock response",
      createdAt: 200,
    };
    const view = renderWithI18n(
      <MessageList
        messages={[userMessage, assistantMessage]}
        userName="Eric"
        isResponding={false}
      />,
    );
    const list = screen.getByTestId("message-list");
    Object.defineProperty(list, "scrollHeight", { configurable: true, value: 240 });
    Object.defineProperty(list, "clientHeight", { configurable: true, value: 100 });
    list.scrollTop = 0;
    fireEvent.scroll(list);

    view.rerender(
      <AppI18nProvider>
        <MessageList
          messages={[
            userMessage,
            assistantMessage,
            { id: "user-2", role: "user", content: "Follow-up", createdAt: 300 },
          ]}
          userName="Eric"
          isResponding={false}
        />
      </AppI18nProvider>,
    );

    expect(list.scrollTop).toBe(240);
  });
});
