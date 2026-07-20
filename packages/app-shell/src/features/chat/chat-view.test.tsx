import { render, screen } from "@testing-library/react";
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
});
