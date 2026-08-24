import { createRef } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Editor } from "@tiptap/core";
import { describe, expect, it, vi } from "vitest";
import {
  createComposerExtensions,
  documentPlainText,
} from "@ora/editor/composer";
import { PlatformProvider } from "@ora/app-shell/platform";
import { ComposerEditor, type ComposerEditorHandle } from "./composer-editor";
import { createStubPlatform } from "../../test/stub-platform";

function composerText(element: HTMLElement): string {
  return element.dataset.composerText ?? "";
}

describe("ComposerEditor", () => {
  it("sends on Enter and inserts a newline on Shift+Enter", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(
      <ComposerEditor
        ariaLabel="Message"
        placeholder="Type"
        onSubmit={onSubmit}
      />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("hello");
    await user.keyboard("{Enter}");
    expect(onSubmit).toHaveBeenCalledTimes(1);

    onSubmit.mockClear();
    await user.keyboard("first");
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    await user.keyboard("second");
    expect(onSubmit).not.toHaveBeenCalled();
    expect(composerText(textbox)).toMatch(/first/);
    expect(composerText(textbox)).toMatch(/second/);
  });

  it("moves the caret forward across consecutive Shift+Enter newlines", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("first");
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    await user.keyboard("last");

    expect(composerText(textbox)).toBe("first\n\n\nlast");
  });

  it("preserves line breaks when plain multiline text is pasted", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.paste(
      "first plain line\nsecond plain line\n\n\nthird plain line",
    );

    expect(composerText(textbox)).toBe(
      "first plain line\nsecond plain line\n\n\nthird plain line",
    );
  });

  it("turns a markdown heading prefix into a heading node", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("# Hello");

    const heading = textbox.querySelector("h1");
    expect(heading).not.toBeNull();
    expect(heading?.textContent).toBe("Hello");
    expect(textbox.textContent).not.toContain("#");

    await user.keyboard("{Enter}");
    await user.keyboard("body");
    expect(textbox.querySelector("h1")?.textContent).toBe("Hello");
    expect(textbox.querySelector("p")?.textContent).toBe("body");
  });

  it("turns markdown and pasted URLs into exclusive underlined links", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.paste("[haha](http://www.baidu.com)");

    const markdownLink = textbox.querySelector("a");
    expect(markdownLink).not.toBeNull();
    expect(markdownLink?.textContent).toBe("haha");
    expect(markdownLink?.getAttribute("href")).toBe("http://www.baidu.com");

    await user.keyboard(" more");
    expect(markdownLink?.textContent).toBe("haha");
    expect(composerText(textbox)).toMatch(/more/);

    await user.keyboard("{Shift>}{Enter}{/Shift}");
    await user.paste("https://example.com");
    const pasted = [...textbox.querySelectorAll("a")].at(-1);
    expect(pasted?.getAttribute("href")).toMatch(/https:\/\/example.com/);
    await user.keyboard(" after");
    expect(pasted?.textContent).not.toContain("after");
  });

  it("inserts file chips that serialize to backtick path ranges", async () => {
    const editorRef = createRef<ComposerEditorHandle>();
    render(
      <ComposerEditor ref={editorRef} ariaLabel="Message" onSubmit={vi.fn()} />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.insertFileChips([
        { path: "src/app.ts", startLine: 4, endLine: 12 },
      ]);
    });

    await waitFor(() =>
      expect(textbox.querySelector("[data-composer-file]")).not.toBeNull(),
    );
    const chip = textbox.querySelector("[data-composer-file]");
    expect(chip).toHaveAttribute("data-start-line", "4");
    expect(chip).toHaveAttribute("data-end-line", "12");
    expect(chip).toHaveAttribute("title", "src/app.ts:4-12");
    expect(composerText(textbox)).toContain("`src/app.ts:4-12`");
    expect(textbox.textContent).toContain("app.ts");
    expect(textbox.textContent).toContain("L4-12");
  });

  it("paints file chips inside a spanning text selection", async () => {
    const user = userEvent.setup();
    const editorRef = createRef<ComposerEditorHandle>();
    render(
      <ComposerEditor ref={editorRef} ariaLabel="Message" onSubmit={vi.fn()} />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.insertFileChips([{ path: "src/app.ts" }]);
    });
    await waitFor(() =>
      expect(textbox.querySelector("[data-composer-file]")).not.toBeNull(),
    );

    await user.click(textbox);
    await user.keyboard("{Control>}a{/Control}");

    await waitFor(() =>
      expect(textbox.querySelector("[data-chip-selected]")).not.toBeNull(),
    );
  });

  it("serializes quoted snippets as citation fences for the agent", async () => {
    const editorRef = createRef<ComposerEditorHandle>();
    render(
      <ComposerEditor ref={editorRef} ariaLabel="Message" onSubmit={vi.fn()} />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.insertFileChips([
        {
          path: "src/app.ts",
          startLine: 4,
          endLine: 5,
          snippet: "const a = 1;\nconst b = 2;",
        },
      ]);
    });

    await waitFor(() =>
      expect(textbox.querySelector("[data-composer-file]")).not.toBeNull(),
    );
    expect(composerText(textbox)).toContain("```4:5:src/app.ts");
    expect(composerText(textbox)).toContain("const a = 1;");
    expect(textbox.textContent).toContain("L4-5");
  });

  it("serializes diff-gutter quotes as unified diff fences for the agent", async () => {
    const editorRef = createRef<ComposerEditorHandle>();
    render(
      <ComposerEditor ref={editorRef} ariaLabel="Message" onSubmit={vi.fn()} />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.insertFileChips([
        {
          path: "src/example.ts",
          startLine: 1,
          endLine: 2,
          snippet: " keep\n+new line",
          origin: "diff",
          diffSide: "new",
        },
      ]);
    });

    await waitFor(() =>
      expect(textbox.querySelector("[data-composer-file]")).not.toBeNull(),
    );
    expect(composerText(textbox)).toContain(
      "diff --git a/src/example.ts b/src/example.ts",
    );
    expect(composerText(textbox)).toContain("quoted from git diff (new side)");
    expect(composerText(textbox)).toContain("+new line");
    expect(textbox.textContent).toContain("L1-2");
  });

  it("replaceDocument restores chips from TipTap JSON without markdown round-trip", async () => {
    const editorRef = createRef<ComposerEditorHandle>();
    render(
      <ComposerEditor ref={editorRef} ariaLabel="Message" onSubmit={vi.fn()} />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.replaceDocument({
        type: "doc",
        content: [
          {
            type: "paragraph",
            content: [
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
            ],
          },
        ],
      });
    });

    await waitFor(() => {
      const dir = textbox.querySelector("[data-composer-file='src']");
      expect(dir).not.toBeNull();
      expect(dir).toHaveAttribute("data-kind", "directory");
      expect(
        textbox.querySelector("[data-prompt-token='command']"),
      ).not.toBeNull();
      expect(textbox.querySelector("code")).toBeNull();
    });
    expect(editorRef.current?.getJSON().content?.[0]).toMatchObject({
      type: "paragraph",
      content: [
        {
          type: "composerFile",
          attrs: { path: "src", kind: "directory" },
        },
        { type: "text", text: " " },
        {
          type: "promptToken",
          attrs: { kind: "command", name: "test" },
        },
      ],
    });
  });

  it("appendText keeps slash-command chips instead of round-tripping through Markdown", async () => {
    const editorRef = createRef<ComposerEditorHandle>();
    render(
      <ComposerEditor ref={editorRef} ariaLabel="Message" onSubmit={vi.fn()} />,
    );

    act(() => {
      editorRef.current?.replaceDocument({
        type: "doc",
        content: [
          {
            type: "paragraph",
            content: [
              {
                type: "promptToken",
                attrs: { kind: "command", name: "test" },
              },
            ],
          },
        ],
      });
    });
    act(() => {
      editorRef.current?.appendText("more");
    });

    await waitFor(() => {
      expect(
        editorRef.current
          ?.getJSON()
          .content?.some((block) =>
            (block.content ?? []).some(
              (node) =>
                node.type === "promptToken" &&
                node.attrs?.kind === "command" &&
                node.attrs?.name === "test",
            ),
          ),
      ).toBe(true);
    });
    expect(editorRef.current?.getText()).toMatch(/more/);
  });

  it("inserts a file chip at the caret without jumping to the document end", () => {
    const editor = new Editor({
      extensions: createComposerExtensions({ placeholder: "Type" }),
      content: {
        type: "doc",
        content: [
          {
            type: "paragraph",
            content: [{ type: "text", text: "head trail" }],
          },
        ],
      },
    });
    // Position after "head " (doc position 1 is start of paragraph text).
    editor
      .chain()
      .setTextSelection(1 + "head ".length)
      .run();
    editor.commands.insertComposerFiles([{ path: "src/mid.ts" }]);
    editor.commands.insertContent("X");

    expect(documentPlainText(editor.state.doc)).toBe(
      "head `src/mid.ts` Xtrail",
    );
    editor.destroy();
  });

  it("keeps adjacent file chips free of selectable separator spaces", () => {
    const editor = new Editor({
      extensions: createComposerExtensions({ placeholder: "Type" }),
      content: {
        type: "doc",
        content: [{ type: "paragraph" }],
      },
    });
    editor.commands.insertComposerFiles([
      { path: "hack.svg" },
      { path: "copy.svg" },
    ]);

    const paragraph = editor.state.doc.firstChild;
    expect(paragraph?.childCount).toBe(3);
    expect(paragraph?.child(0).type.name).toBe("composerFile");
    expect(paragraph?.child(1).type.name).toBe("composerFile");
    expect(paragraph?.child(2).isText).toBe(true);
    expect(paragraph?.child(2).text).toBe(" ");
    expect(documentPlainText(editor.state.doc)).toBe("`hack.svg` `copy.svg` ");
    editor.destroy();
  });

  it("restores documentPlainText as formatted nodes instead of leftover markers", () => {
    render(
      <ComposerEditor
        ariaLabel="Message"
        initialText={"# Title\n**bold**\n- item"}
        onSubmit={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    expect(textbox.querySelector("h1")?.textContent).toBe("Title");
    expect(textbox.querySelector("strong, b")?.textContent).toBe("bold");
    expect(textbox.querySelector("ul")?.textContent).toMatch(/item/);
    expect(textbox.textContent).not.toContain("#");
    expect(textbox.textContent).not.toContain("**");
  });

  it("keeps HTML tags as text when restoring a draft", () => {
    render(
      <ComposerEditor
        ariaLabel="Message"
        initialText="<script>alert(1)</script>"
        onSubmit={vi.fn()}
      />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    expect(textbox.querySelector("script")).toBeNull();
    expect(textbox.textContent).toContain("<script>alert(1)</script>");
  });

  it("replaceText parses composer markdown the same way as initialText", () => {
    const editorRef = createRef<ComposerEditorHandle>();
    render(
      <ComposerEditor ref={editorRef} ariaLabel="Message" onSubmit={vi.fn()} />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.replaceText("**bold**\n- item");
    });

    expect(textbox.querySelector("strong, b")?.textContent).toBe("bold");
    expect(textbox.querySelector("ul")?.textContent).toMatch(/item/);
    expect(textbox.textContent).not.toContain("**");
  });

  it("pastes adjacent bold and italic that share a middle ***", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.paste("**加粗***倾斜*");
    expect(textbox.querySelector("strong, b")?.textContent).toBe("加粗");
    expect(textbox.querySelector("em, i")?.textContent).toBe("倾斜");
    expect(textbox.textContent).not.toContain("*");
  });

  it("converts leftover marks after the opener is typed in front and a space follows", async () => {
    const user = userEvent.setup();
    const editorRef = createRef<ComposerEditorHandle>();
    render(
      <ComposerEditor ref={editorRef} ariaLabel="Message" onSubmit={vi.fn()} />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("bold**");
    expect(textbox.querySelector("strong, b")).toBeNull();
    act(() => {
      editorRef.current?.focus({ at: "start" });
    });
    await user.keyboard("**");
    expect(textbox.querySelector("strong, b")).toBeNull();
    act(() => {
      editorRef.current?.focus({ at: "end" });
    });
    await user.keyboard(" ");
    expect(textbox.querySelector("strong, b")?.textContent).toBe("bold");
    expect(textbox.textContent).not.toContain("*");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("倾斜*");
    act(() => {
      editorRef.current?.focus({ at: "start" });
    });
    await user.keyboard("*");
    expect(textbox.querySelector("em, i")).toBeNull();
    act(() => {
      editorRef.current?.focus({ at: "end" });
    });
    await user.keyboard(" ");
    expect(textbox.querySelector("em, i")?.textContent).toBe("倾斜");
    expect(textbox.textContent).not.toContain("*");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("加粗***倾斜*");
    act(() => {
      editorRef.current?.focus({ at: "start" });
    });
    await user.keyboard("**");
    expect(textbox.querySelector("strong, b")).toBeNull();
    act(() => {
      editorRef.current?.focus({ at: "end" });
    });
    await user.keyboard(" ");
    expect(textbox.querySelector("strong, b")?.textContent).toBe("加粗");
    expect(textbox.querySelector("em, i")?.textContent).toBe("倾斜");
    expect(textbox.textContent).not.toContain("*");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("a==");
    act(() => {
      editorRef.current?.focus({ at: "start" });
    });
    await user.keyboard("==");
    expect(textbox.querySelector("mark")).toBeNull();
    act(() => {
      editorRef.current?.focus({ at: "end" });
    });
    await user.keyboard(" ");
    expect(textbox.querySelector("mark")?.textContent).toBe("a");
    expect(textbox.textContent).not.toContain("=");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("out~~");
    act(() => {
      editorRef.current?.focus({ at: "start" });
    });
    await user.keyboard("~~");
    expect(textbox.querySelector("s, del")).toBeNull();
    act(() => {
      editorRef.current?.focus({ at: "end" });
    });
    await user.keyboard(" ");
    expect(textbox.querySelector("s, del")?.textContent).toBe("out");
    expect(textbox.textContent).not.toContain("~");
  });

  it("keeps typing at the start of converted marks inside the mark", async () => {
    const user = userEvent.setup();
    const editorRef = createRef<ComposerEditorHandle>();
    render(
      <ComposerEditor ref={editorRef} ariaLabel="Message" onSubmit={vi.fn()} />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("**bold**");
    expect(textbox.querySelector("strong, b")?.textContent).toBe("bold");
    act(() => {
      editorRef.current?.focus({ at: "start" });
    });
    await user.keyboard("pre");
    expect(textbox.querySelector("strong, b")?.textContent).toBe("prebold");
    expect(textbox.textContent).not.toContain("*");
  });

  it("turns an existing line into a heading when the prefix is typed at the start", async () => {
    const user = userEvent.setup();
    const editorRef = createRef<ComposerEditorHandle>();
    render(
      <ComposerEditor ref={editorRef} ariaLabel="Message" onSubmit={vi.fn()} />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("Title");
    act(() => {
      editorRef.current?.focus({ at: "start" });
    });
    await user.keyboard("# ");
    expect(textbox.querySelector("h1")?.textContent).toBe("Title");
    expect(textbox.textContent).not.toContain("#");
  });

  it("turns a quote prefix into a blockquote", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("> quoted");

    expect(textbox.querySelector("blockquote")).not.toBeNull();
    expect(textbox.querySelector("blockquote")?.textContent).toMatch(/quoted/);
  });

  it("leaves a quote on Enter and newlines inside it on Shift+Enter", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<ComposerEditor ariaLabel="Message" onSubmit={onSubmit} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("> hello");
    expect(textbox.querySelector("blockquote")).not.toBeNull();

    await user.keyboard("{Shift>}{Enter}{/Shift}");
    await user.keyboard("still quoted");
    expect(onSubmit).not.toHaveBeenCalled();
    expect(textbox.querySelectorAll("blockquote p").length).toBeGreaterThan(1);

    await user.keyboard("{Enter}");
    expect(onSubmit).not.toHaveBeenCalled();
    await user.keyboard("body");
    expect(textbox.querySelector("blockquote")?.textContent).toMatch(/hello/);
    expect(textbox.querySelector("blockquote")?.textContent).toMatch(
      /still quoted/,
    );
    expect(textbox.querySelector("blockquote")?.textContent).not.toMatch(
      /body/,
    );
    expect(
      [...textbox.querySelectorAll("p")].some((node) =>
        node.textContent?.includes("body"),
      ),
    ).toBe(true);
  });

  it("lifts an empty quote on Shift+Enter instead of inserting another paragraph inside it", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<ComposerEditor ariaLabel="Message" onSubmit={onSubmit} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("> hello");
    expect(textbox.querySelector("blockquote")).not.toBeNull();
    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    expect(onSubmit).not.toHaveBeenCalled();
    expect(textbox.querySelector("blockquote")).toBeNull();
  });

  it("leaves a list on Enter and adds an item on Shift+Enter", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<ComposerEditor ariaLabel="Message" onSubmit={onSubmit} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("- first");
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    await user.keyboard("second");
    expect(onSubmit).not.toHaveBeenCalled();
    expect(textbox.querySelectorAll("li")).toHaveLength(2);

    await user.keyboard("{Enter}");
    await user.keyboard("body");
    expect(onSubmit).not.toHaveBeenCalled();
    expect(textbox.querySelector("ul")?.textContent).toMatch(/first/);
    expect(textbox.querySelector("ul")?.textContent).toMatch(/second/);
    expect(textbox.querySelector("ul")?.textContent).not.toMatch(/body/);
  });

  it("keeps a chip-only list item on Enter instead of lifting it as empty", async () => {
    const user = userEvent.setup();
    const editorRef = createRef<ComposerEditorHandle>();
    const onSubmit = vi.fn();
    render(
      <ComposerEditor
        ref={editorRef}
        ariaLabel="Message"
        onSubmit={onSubmit}
      />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    act(() => {
      editorRef.current?.replaceDocument({
        type: "doc",
        content: [
          {
            type: "bulletList",
            content: [
              {
                type: "listItem",
                content: [
                  {
                    type: "paragraph",
                    content: [
                      {
                        type: "composerFile",
                        attrs: {
                          path: "src/app.ts",
                          startLine: null,
                          endLine: null,
                          kind: "file",
                        },
                      },
                    ],
                  },
                ],
              },
            ],
          },
        ],
      });
    });
    await waitFor(() =>
      expect(textbox.querySelector("[data-composer-file]")).not.toBeNull(),
    );
    act(() => {
      editorRef.current?.focus({ at: "end" });
    });
    await user.keyboard("{Enter}");
    expect(onSubmit).not.toHaveBeenCalled();
    expect(textbox.querySelector("ul")).not.toBeNull();
    expect(textbox.querySelector("[data-composer-file]")).not.toBeNull();
  });

  it("leaves a heading on Enter without sending", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<ComposerEditor ariaLabel="Message" onSubmit={onSubmit} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("# Title{Enter}more");

    expect(onSubmit).not.toHaveBeenCalled();
    expect(textbox.querySelector("h1")?.textContent).toMatch(/Title/);
    expect(textbox.querySelector("p")?.textContent).toMatch(/more/);
  });

  it("does not submit while an IME composition is active", async () => {
    const onSubmit = vi.fn();
    render(<ComposerEditor ariaLabel="Message" onSubmit={onSubmit} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });
    textbox.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Enter",
        bubbles: true,
        cancelable: true,
        isComposing: true,
      }),
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("moves existing text down when Shift+Enter is pressed at the start of a line", async () => {
    const user = userEvent.setup();
    const editorRef = createRef<ComposerEditorHandle>();
    render(
      <ComposerEditor ref={editorRef} ariaLabel="Message" onSubmit={vi.fn()} />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("hello");
    act(() => {
      editorRef.current?.focus({ at: "start" });
    });
    await user.keyboard("{Shift>}{Enter}{/Shift}");

    expect(composerText(textbox)).toBe("\nhello");
  });

  it("turns three backticks into a fenced code block instead of sending", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<ComposerEditor ariaLabel="Message" onSubmit={onSubmit} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("```");
    expect(textbox.querySelector("pre")).toBeNull();
    await user.keyboard("{Shift>}{Enter}{/Shift}");

    expect(onSubmit).not.toHaveBeenCalled();
    expect(textbox.querySelector("pre")).not.toBeNull();
  });

  it("opens a fence on Enter after three backticks instead of sending", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<ComposerEditor ariaLabel="Message" onSubmit={onSubmit} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("```{Enter}");

    expect(onSubmit).not.toHaveBeenCalled();
    expect(textbox.querySelector("pre")).not.toBeNull();
  });

  it("keeps ```C++ as a language fence, newlines on Shift+Enter, and leaves on Enter", async () => {
    const user = userEvent.setup();
    const onSubmit = vi.fn();
    render(<ComposerEditor ariaLabel="Message" onSubmit={onSubmit} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("```C++");
    expect(textbox.querySelector("pre")).toBeNull();
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    expect(onSubmit).not.toHaveBeenCalled();
    expect(textbox.querySelector("pre")).not.toBeNull();

    await user.keyboard("int x;");
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    await user.keyboard("int y;");
    await user.keyboard("{Enter}");
    await user.keyboard("done");

    expect(onSubmit).not.toHaveBeenCalled();
    expect(textbox.querySelector("pre")?.textContent).toMatch(/int x;/);
    expect(textbox.querySelector("pre")?.textContent).toMatch(/int y;/);
    expect(textbox.querySelector("p")?.textContent).toMatch(/done/);
    expect(composerText(textbox)).toMatch(/```C\+\+/);
  });

  it("opens a language fence when a space follows the info string", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("```C++ ");

    expect(textbox.querySelector("pre")).not.toBeNull();
    expect(composerText(textbox)).toMatch(/```C\+\+/);
  });

  it("supports CommonMark basics: headings 1-6, lists, emphasis, and strike", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    for (const level of [1, 2, 3, 4, 5, 6] as const) {
      await user.keyboard(`${"#".repeat(level)} Title ${level}`);
      expect(textbox.querySelector(`h${level}`)?.textContent).toBe(
        `Title ${level}`,
      );
      await user.keyboard("{Enter}");
    }

    await user.keyboard("- bullet");
    expect(textbox.querySelector("ul")).not.toBeNull();
    expect(textbox.querySelector("ul")?.textContent).toMatch(/bullet/);

    await user.keyboard("{Enter}");
    await user.keyboard("1. numbered");
    expect(textbox.querySelector("ol")).not.toBeNull();

    await user.keyboard("{Enter}");
    await user.keyboard("**bold** *em* ~~out~~ `code`");
    expect(textbox.querySelector("strong, b")).not.toBeNull();
    expect(textbox.querySelector("em, i")).not.toBeNull();
    expect(textbox.querySelector("s, del")).not.toBeNull();
    expect(textbox.querySelector("code")).not.toBeNull();
  });

  it("turns task-list, rule, and Ctrl+B markdown into the matching nodes", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("- [[ ] todo");
    const taskList = textbox.querySelector('ul[data-type="taskList"]');
    expect(taskList).not.toBeNull();
    expect(taskList?.querySelector("ul")).toBeNull();
    expect(textbox.querySelector("ul:not([data-type='taskList'])")).toBeNull();
    expect(taskList?.querySelector("input[type='checkbox']")).not.toBeNull();
    expect(taskList?.textContent).toMatch(/todo/);
    expect(taskList?.textContent).not.toMatch(/\[/);

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("[[ ] from body");
    expect(textbox.querySelector('ul[data-type="taskList"]')).not.toBeNull();
    expect(
      textbox.querySelector('ul[data-type="taskList"]')?.textContent,
    ).toMatch(/from body/);

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("---");
    expect(
      textbox.querySelector("hr, [data-type='horizontalRule']"),
    ).not.toBeNull();

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("hello");
    await user.keyboard("{Control>}a{/Control}");
    await user.keyboard("{Control>}b{/Control}");
    expect(textbox.querySelector("strong, b")).not.toBeNull();
  });

  it("pastes a Markdown document into headings, lists, marks, and fences", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.paste(
      [
        "# Title",
        "> quoted",
        "- bullet",
        "1. numbered",
        "- [x] done",
        "---",
        "```ts",
        "const n = 1;",
        "```",
        "**bold** *em* ~~out~~ `code` ==hi== [Docs](https://example.com)",
      ].join("\n"),
    );

    expect(textbox.querySelector("h1")?.textContent).toBe("Title");
    expect(textbox.querySelector("blockquote")?.textContent).toMatch(/quoted/);
    expect(textbox.querySelector("ul")?.textContent).toMatch(/bullet/);
    expect(textbox.querySelector("ol")?.textContent).toMatch(/numbered/);
    expect(textbox.querySelector('ul[data-type="taskList"]')).not.toBeNull();
    expect(
      textbox.querySelector("hr, [data-type='horizontalRule']"),
    ).not.toBeNull();
    expect(textbox.querySelector("pre")?.textContent).toMatch(/const n = 1;/);
    expect(textbox.querySelector("strong, b")).not.toBeNull();
    expect(textbox.querySelector("em, i")).not.toBeNull();
    expect(textbox.querySelector("s, del")).not.toBeNull();
    expect(textbox.querySelector("code")).not.toBeNull();
    expect(textbox.querySelector("mark")).not.toBeNull();
    expect(textbox.querySelector("a")?.getAttribute("href")).toBe(
      "https://example.com",
    );
    expect(composerText(textbox)).toContain("**bold**");
    expect(composerText(textbox)).toContain("[Docs](https://example.com)");
  });

  it("pastes nested quotes, lists inside quotes, and titled links", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.paste(
      [
        "> outer",
        "> > inner",
        "> - listed",
        '[Docs](https://example.com "hover")',
      ].join("\n"),
    );

    expect(textbox.querySelector("blockquote")?.textContent).toMatch(/outer/);
    expect(textbox.querySelector("blockquote blockquote")?.textContent).toBe(
      "inner",
    );
    expect(textbox.querySelector("blockquote ul")?.textContent).toMatch(
      /listed/,
    );
    const titled = textbox.querySelector("a");
    expect(titled?.getAttribute("href")).toBe("https://example.com");
    expect(titled?.getAttribute("title")).toBe("hover");
    expect(titled?.textContent).toBe("Docs");
  });

  it("covers the remaining typed Markdown surface without leftover delimiters", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("+ plus");
    expect(
      textbox.querySelector("ul:not([data-type='taskList'])"),
    ).not.toBeNull();
    expect(textbox.querySelector("ul")?.textContent).toMatch(/plus/);

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("* star");
    expect(
      textbox.querySelector("ul:not([data-type='taskList'])"),
    ).not.toBeNull();
    expect(textbox.querySelector("ul")?.textContent).toMatch(/star/);

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("- [[x] done");
    const checked = textbox.querySelector(
      'ul[data-type="taskList"] li[data-checked="true"]',
    );
    expect(checked).not.toBeNull();
    expect(checked?.textContent).toMatch(/done/);
    expect(textbox.querySelector("ul:not([data-type='taskList'])")).toBeNull();

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("***both*** after");
    const boldItalic = textbox.querySelector("strong, b");
    expect(boldItalic?.textContent).toBe("both");
    expect(textbox.querySelector("em, i")?.textContent).toBe("both");
    expect(boldItalic?.textContent).not.toContain("after");
    expect(textbox.textContent).not.toContain("*");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("**bold** after");
    expect(textbox.querySelector("strong, b")?.textContent).toBe("bold");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("__bold__ _em_");
    expect(textbox.querySelector("strong, b")?.textContent).toBe("bold");
    expect(textbox.querySelector("em, i")?.textContent).toBe("em");
    expect(textbox.textContent).not.toContain("_");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("sa~~d~~ more");
    const strike = textbox.querySelector("s, del");
    expect(strike?.textContent).toBe("d");
    expect(textbox.textContent).not.toContain("~");
    expect(strike?.textContent).not.toContain("sa");
    expect(strike?.textContent).not.toContain("more");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("==hi== after");
    const mark = textbox.querySelector("mark");
    expect(mark?.textContent).toBe("hi");
    expect(mark?.textContent).not.toContain("=");
    expect(mark?.textContent).not.toContain("after");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("`code` after");
    const code = textbox.querySelector("code");
    expect(code?.textContent).toBe("code");
    expect(code?.textContent).not.toContain("after");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("# **Title**");
    expect(textbox.querySelector("h1")?.textContent).toBe("Title");
    expect(textbox.querySelector("h1 strong, h1 b")).not.toBeNull();
  });

  it("applies bold and italic next to CJK without requiring an ASCII space", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("你好**等等**");
    expect(textbox.querySelector("strong, b")?.textContent).toBe("等等");
    expect(textbox.textContent).not.toContain("*");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("等等你好 *d*");
    expect(textbox.querySelector("em, i")?.textContent).toBe("d");
    expect(textbox.textContent).not.toContain("*");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("哈哈**的哈**");
    expect(textbox.querySelector("strong, b")?.textContent).toBe("的哈");
    expect(textbox.textContent).not.toContain("*");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("2 * 3 * 4");
    expect(textbox.querySelector("em, i")).toBeNull();
    expect(textbox.textContent).toContain("*");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("ddssd**d **");
    expect(textbox.querySelector("strong, b")).toBeNull();
  });

  it("underlines with Ctrl+U", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("hello");
    await user.keyboard("{Control>}a{/Control}");
    await user.keyboard("{Control>}u{/Control}");
    expect(textbox.querySelector("u")).not.toBeNull();
    await user.keyboard("{ArrowRight} after");
    expect(textbox.querySelector("u")?.textContent).toBe("hello");
  });

  it("turns ==highlight== into a highlighter mark and hides the delimiters", async () => {
    const user = userEvent.setup();
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("==hi==");
    expect(textbox.querySelector("mark")?.textContent).toBe("hi");
    expect(textbox.textContent).not.toContain("=");
    expect(composerText(textbox)).toContain("==hi==");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("==高亮==");
    expect(textbox.querySelector("mark")?.textContent).toBe("高亮");
    expect(textbox.textContent).not.toContain("=");

    await user.keyboard("{Control>}a{/Control}{Backspace}");
    await user.keyboard("- ==高亮==");
    expect(textbox.querySelector("ul")).not.toBeNull();
    expect(textbox.querySelector("mark")?.textContent).toBe("高亮");
    expect(textbox.textContent).not.toContain("=");
  });

  it("opens underlined links on click", async () => {
    const user = userEvent.setup();
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    render(<ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />);
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.paste("[haha](http://www.baidu.com)");
    const link = textbox.querySelector("a");
    expect(link).not.toBeNull();
    await user.click(link!);

    expect(open).toHaveBeenCalled();
    const openedUrl = String(open.mock.calls[0]?.[0] ?? "");
    expect(openedUrl).toContain("baidu.com");
    open.mockRestore();
  });

  it("opens underlined links through the platform on the first press", async () => {
    const user = userEvent.setup();
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    const openExternalUrl = vi.fn().mockResolvedValue(undefined);
    render(
      <PlatformProvider adapter={{ ...createStubPlatform(), openExternalUrl }}>
        <ComposerEditor ariaLabel="Message" onSubmit={vi.fn()} />
      </PlatformProvider>,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.paste("[haha](http://www.baidu.com)");
    const link = textbox.querySelector("a");
    expect(link).not.toBeNull();
    await user.click(link!);

    expect(openExternalUrl).toHaveBeenCalled();
    const openedUrl = String(openExternalUrl.mock.calls[0]?.[0] ?? "");
    expect(openedUrl).toContain("baidu.com");
    expect(open).not.toHaveBeenCalled();
    open.mockRestore();
  });

  it("reports slash queries after existing text and keeps that text when inserting a command", async () => {
    const user = userEvent.setup();
    const editorRef = createRef<ComposerEditorHandle>();
    const onQueryChange = vi.fn();
    render(
      <ComposerEditor
        ref={editorRef}
        ariaLabel="Message"
        onSubmit={vi.fn()}
        onQueryChange={onQueryChange}
      />,
    );
    const textbox = screen.getByRole("textbox", { name: "Message" });

    await user.click(textbox);
    await user.keyboard("hello /re");
    expect(onQueryChange).toHaveBeenCalledWith(
      expect.objectContaining({ slashQuery: "re", isBlank: false }),
    );

    act(() => {
      editorRef.current?.insertPromptToken("command", "review");
    });
    expect(composerText(textbox)).toBe("hello /review ");
    expect(textbox.querySelector(".composer-mention")?.textContent).toBe(
      "/review",
    );
    expect(textbox.querySelector(".composer-chip")).toBeNull();
  });
});
