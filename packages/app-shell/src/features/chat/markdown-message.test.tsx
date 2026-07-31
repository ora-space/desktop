import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { MarkdownMessage } from "./markdown-message";

/** Renders Markdown with the production translation provider used by code controls. */
function renderMarkdown(content: string) {
  return render(<AppI18nProvider><MarkdownMessage content={content} /></AppI18nProvider>);
}

describe("MarkdownMessage", () => {
  it("renders GitHub-flavored Markdown with semantic elements", () => {
    render(<MarkdownMessage content={"## Result\n\n- one\n- two\n\n| Name | Value |\n| --- | --- |\n| Ora | IDE |\n\n`const ready = true;`"} />);

    expect(screen.getByRole("heading", { level: 2, name: "Result" }).closest("[data-selectable]")).not.toBeNull();
    expect(screen.getByRole("list")).toHaveTextContent("one");
    expect(screen.getByRole("table")).toHaveTextContent("Ora");
    expect(screen.getByText("const ready = true;")).toHaveClass("font-mono");
  });

  it("keeps links safe and does not interpret raw HTML", () => {
    render(<MarkdownMessage content={'[Documentation](https://example.com)\n\n<script>alert("unsafe")</script>'} />);

    expect(screen.getByRole("link", { name: "Documentation" })).toHaveAttribute("rel", "noopener noreferrer");
    expect(screen.queryByRole("script")).toBeNull();
    expect(screen.getByText(/<script>/)).toBeInTheDocument();
  });

  it("renders a document wrapped in a Markdown code fence", () => {
    render(<MarkdownMessage content={"```markdown\n# Wrapped result\n\n**Rendered**, not code.\n```"} />);

    expect(screen.getByRole("heading", { level: 1, name: "Wrapped result" })).toBeInTheDocument();
    expect(screen.getByText("Rendered").tagName).toBe("STRONG");
    expect(screen.queryByText(/# Wrapped result/)).toBeNull();
  });

  it("keeps streamed markdown visible while batching expensive parsing", () => {
    render(<MarkdownMessage content={"# Live heading\n\nStill streaming."} streaming />);

    expect(screen.getByRole("heading", { level: 1, name: "Live heading" })).toBeInTheDocument();
    expect(screen.getByText("Still streaming.")).toBeInTheDocument();
  });

  it.each(["*", "**", "***", "_", "__", "___", "~", "~~", "`", "``"])(
    "withholds an ambiguous %s boundary until prose arrives",
    (delimiter) => {
      const view = render(<MarkdownMessage content={delimiter} streaming />);

      expect(view.container).not.toHaveTextContent(delimiter);
    },
  );

  it("reveals a buffered emphasis boundary and its first prose in one render", async () => {
    const view = render(<MarkdownMessage content="Prefix **" streaming />);

    expect(view.container).toHaveTextContent("Prefix");
    expect(view.container).not.toHaveTextContent("**");

    view.rerender(<MarkdownMessage content="Prefix **natural" streaming />);

    await waitFor(() => expect(screen.getByText("natural").closest("strong")).not.toBeNull());
    expect(view.container).toHaveTextContent("Prefix natural");
  });

  it("flushes an ambiguous literal marker when streaming completes", () => {
    const view = render(<MarkdownMessage content="Literal *" streaming />);
    expect(view.container).toHaveTextContent("Literal");
    expect(view.container).not.toHaveTextContent("Literal *");

    view.rerender(<MarkdownMessage content="Literal *" streaming={false} />);

    expect(view.container).toHaveTextContent("Literal *");
  });

  it("withholds empty block markers without delaying completed block content", async () => {
    const view = render(<MarkdownMessage content="##" streaming />);

    expect(view.container).not.toHaveTextContent("##");

    view.rerender(<MarkdownMessage content="## Live heading" streaming />);

    await waitFor(() => {
      expect(screen.getByRole("heading", { level: 2, name: "Live heading" })).toBeInTheDocument();
    });
  });

  it("preserves trailing markers that belong to code or escaped prose", () => {
    const code = render(<MarkdownMessage content={"`literal *`"} streaming />);
    expect(screen.getByText("literal *").closest("code")).not.toBeNull();
    code.unmount();

    render(<MarkdownMessage content={"Escaped \\* and \\#"} streaming />);
    expect(screen.getByText("Escaped * and #")).toBeInTheDocument();
  });

  it("releases literal marker characters as soon as following prose resolves them", () => {
    render(<MarkdownMessage content="Use * as a wildcard and # as a heading marker." streaming />);

    expect(screen.getByText("Use * as a wildcard and # as a heading marker.")).toBeInTheDocument();
  });

  it("renders growing GFM block structures without waiting for completion", () => {
    render(
      <MarkdownMessage
        content={"- first item\n- growing item\n\n> Live quote\n\n| Name | Value |\n| --- | --- |\n| Ora | stream"}
        streaming
      />,
    );

    expect(screen.getByRole("list")).toHaveTextContent("growing item");
    expect(screen.getByText("Live quote").closest("blockquote")).not.toBeNull();
    expect(screen.getByRole("table")).toHaveTextContent("Ora");
    expect(screen.getByRole("table")).toHaveTextContent("stream");
  });

  it.each([
    ["strong", "**Live strong", "strong"],
    ["asterisk emphasis", "*Live emphasis", "em"],
    ["underscore emphasis", "_Live emphasis", "em"],
    ["strikethrough", "~~Live removed", "del"],
    ["inline code", "`Live code", "code"],
  ])("renders unfinished %s immediately with its final styling", (_label, content, selector) => {
    const visibleText = content.replace(/^[*_~`]+/, "");
    const view = render(<MarkdownMessage content={content} streaming />);

    expect(screen.getByText(visibleText).closest(selector)).not.toBeNull();
    expect(view.container).not.toHaveTextContent(content.slice(0, content.length - visibleText.length));
  });

  it("keeps nested unfinished emphasis stable when real delimiters arrive", async () => {
    const view = render(<MarkdownMessage content="***Live bold italic" streaming />);

    expect(screen.getByText("Live bold italic").closest("strong")).not.toBeNull();
    expect(screen.getByText("Live bold italic").closest("em")).not.toBeNull();

    view.rerender(<MarkdownMessage content="***Live bold italic***" streaming />);

    await waitFor(() => {
      expect(screen.getByText("Live bold italic").closest("strong")).not.toBeNull();
      expect(screen.getByText("Live bold italic").closest("em")).not.toBeNull();
    });
    expect(view.container).not.toHaveTextContent("******");
  });

  it("completes partial links without exposing their destination syntax", () => {
    const view = render(<MarkdownMessage content="[Documentation](https://exam" streaming />);

    expect(screen.getByText("Documentation").closest("a")).not.toBeNull();
    expect(view.container).not.toHaveTextContent("https://exam");
  });

  it("does not interpret formatting markers inside streamed inline code", () => {
    render(<MarkdownMessage content="`**literal _markers_`" streaming />);

    const code = screen.getByText("**literal _markers_");
    expect(code.closest("code")).not.toBeNull();
    expect(code.closest("strong")).toBeNull();
    expect(code.closest("em")).toBeNull();
  });

  it("keeps escaped markers literal while streaming", () => {
    render(<MarkdownMessage content={"\\*literal emphasis marker"} streaming />);

    const text = screen.getByText("*literal emphasis marker");
    expect(text.closest("em")).toBeNull();
  });

  it("keeps an unfinished fenced code block visible without parsing its contents as prose", () => {
    render(<MarkdownMessage content={"```typescript\nconst marker = '**literal**';"} streaming />);

    const code = screen.getByText("const marker = '**literal**';");
    expect(code.closest(".markdown-code-block")).not.toBeNull();
    expect(code.closest("strong")).toBeNull();
  });

  it("reveals entity-backed text without dropping newly appended characters", async () => {
    const view = render(<MarkdownMessage content="Stable &amp;" streaming />);

    view.rerender(<MarkdownMessage content="Stable &amp; addition" streaming />);

    await waitFor(() => {
      expect(view.container.querySelector("[data-stream-text-reveal]")?.textContent).toBe(" addition");
    });
    expect(view.container).toHaveTextContent("Stable & addition");
  });

  it("coalesces rapid chunks into one rendered update per animation frame", () => {
    const originalRequestAnimationFrame = Object.getOwnPropertyDescriptor(window, "requestAnimationFrame");
    const originalCancelAnimationFrame = Object.getOwnPropertyDescriptor(window, "cancelAnimationFrame");
    let frameCallback: FrameRequestCallback | undefined;
    const requestAnimationFrame = vi.fn((callback: FrameRequestCallback) => {
      frameCallback = callback;
      return 1;
    });
    Object.defineProperty(window, "requestAnimationFrame", { configurable: true, value: requestAnimationFrame });
    Object.defineProperty(window, "cancelAnimationFrame", { configurable: true, value: vi.fn() });

    try {
      const view = render(<MarkdownMessage content="A" streaming />);
      view.rerender(<MarkdownMessage content="AB" streaming />);
      view.rerender(<MarkdownMessage content="ABC" streaming />);

      expect(requestAnimationFrame).toHaveBeenCalledTimes(1);
      expect(view.container).toHaveTextContent("A");

      act(() => frameCallback?.(16));

      expect(view.container).toHaveTextContent("ABC");
    } finally {
      if (originalRequestAnimationFrame === undefined) Reflect.deleteProperty(window, "requestAnimationFrame");
      else Object.defineProperty(window, "requestAnimationFrame", originalRequestAnimationFrame);
      if (originalCancelAnimationFrame === undefined) Reflect.deleteProperty(window, "cancelAnimationFrame");
      else Object.defineProperty(window, "cancelAnimationFrame", originalCancelAnimationFrame);
    }
  });

  it("reveals only newly streamed prose without reanimating stable text", async () => {
    const view = render(<MarkdownMessage content="Stable" streaming />);

    expect(view.container.querySelector("[data-stream-text-reveal]")).toHaveTextContent("Stable");

    view.rerender(<MarkdownMessage content="Stable addition" streaming />);

    await waitFor(() => {
      expect(view.container.querySelector("[data-stream-text-reveal]")?.textContent).toBe(" addition");
    });
    expect(view.container).toHaveTextContent("Stable addition");
  });

  it("does not animate completed Markdown or code contents", () => {
    const completed = render(<MarkdownMessage content="Complete" />);
    expect(completed.container.querySelector("[data-stream-text-reveal]")).toBeNull();
    completed.unmount();

    const streamingCode = render(<MarkdownMessage content={"```typescript\nconst answer = 42;\n```"} streaming />);
    expect(streamingCode.container.querySelector("[data-stream-text-reveal]")).toBeNull();
  });

  it("uses a crisp opacity-only reveal without blur or movement", () => {
    const originalAnimate = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "animate");
    const originalGetAnimations = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "getAnimations");
    const animation = { addEventListener: vi.fn() } as unknown as Animation;
    const animate = vi.fn(() => animation);
    Object.defineProperty(HTMLElement.prototype, "animate", { configurable: true, value: animate });
    Object.defineProperty(HTMLElement.prototype, "getAnimations", { configurable: true, value: () => [] });

    try {
      render(<MarkdownMessage content="Crisp stream" streaming />);

      expect(animate).toHaveBeenCalledWith(
        [{ opacity: 0.2 }, { opacity: 1 }],
        { duration: 180, easing: "cubic-bezier(0.2, 0, 0, 1)" },
      );
    } finally {
      if (originalAnimate === undefined) Reflect.deleteProperty(HTMLElement.prototype, "animate");
      else Object.defineProperty(HTMLElement.prototype, "animate", originalAnimate);
      if (originalGetAnimations === undefined) Reflect.deleteProperty(HTMLElement.prototype, "getAnimations");
      else Object.defineProperty(HTMLElement.prototype, "getAnimations", originalGetAnimations);
    }
  });

  it("preserves ordinary fenced code blocks", () => {
    render(<MarkdownMessage content={"Example:\n\n```markdown\n# Literal Markdown\n```"} />);

    expect(screen.queryByRole("heading", { name: "Literal Markdown" })).toBeNull();
    expect(screen.getByText("# Literal Markdown")).toBeInTheDocument();
  });

  it("adds VS Code theme colors to known code languages", async () => {
    renderMarkdown("```typescript\nconst answer: number = 42;\n```");

    expect(screen.getByRole("code").closest(".markdown-code-block")).not.toBeNull();
    await waitFor(() => expect(screen.getByText("const")).toHaveClass("shiki-token"));
    expect(screen.getByText("const")).toHaveStyle({ color: "#0000FF" });
    expect(screen.getByText("const")).toHaveStyle({ "--shiki-dark": "#569CD6" });
  });

  it("copies and collapses fenced code without losing its toolbar", async () => {
    const user = userEvent.setup();
    const writeText = vi.spyOn(navigator.clipboard, "writeText");
    renderMarkdown("```typescript\nconst answer = 42;\n```");

    await user.click(screen.getByRole("button", { name: /复制代码|Copy code/ }));
    expect(writeText).toHaveBeenCalledWith("const answer = 42;");
    expect(screen.getByRole("button", { name: /代码已复制|Code copied/ })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /收起代码|Collapse code/ }));
    expect(screen.queryByRole("code")).toBeNull();
    expect(screen.getByText(/已收起 1 行代码|1 line collapsed/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /展开代码|Expand code/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /代码已复制|Code copied/ })).toBeInTheDocument();
  });

  it("provides controls for fenced code without a language label", () => {
    renderMarkdown("```\nfirst line\nsecond line\n```");

    expect(screen.getByText("text")).toBeInTheDocument();
    expect(screen.getByText(/2 行|2 lines/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /复制代码|Copy code/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /收起代码|Collapse code/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /复制代码|Copy code/ }).closest("[data-selection-control]")).not.toBeNull();
  });
});
