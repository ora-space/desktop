import assert from "node:assert/strict";
import test from "node:test";
import { Schema } from "@tiptap/pm/model";
import {
  createComposerExtensions,
  COMPOSER_HEADING_LEVELS,
} from "../src/composer/create-composer-extensions.ts";
import {
  documentPlainText,
  plainTextToComposerContent,
} from "../src/composer/composer-plain-text.ts";
import {
  composerFileLabel,
  composerFilePlainText,
} from "../src/composer/composer-file.ts";
import { parseFenceOpener } from "../src/composer/composer-code-fence.ts";
import { highlightInputMatch } from "../src/composer/composer-highlight.ts";
import { isComposerOpenableUrl } from "../src/composer/composer-link.ts";
import { boldItalicInputMatch } from "../src/composer/composer-marks.ts";

test("composer preset exposes the markdown minimum set plus exclusive chips", () => {
  const names = createComposerExtensions({ placeholder: "Type" }).map(
    (extension) => extension.name,
  );
  assert.deepEqual(names, [
    "starterKit",
    "horizontalRule",
    "bold",
    "italic",
    "strike",
    "code",
    "underline",
    "taskList",
    "taskItem",
    "highlight",
    "link",
    "composerFile",
    "promptToken",
    "composerChipSelection",
    "composerNewline",
    "composerCodeFence",
    "composerMarkdownPaste",
    "composerMarkdownBackfill",
    "composerMarkdownRevert",
    "composerMarkStartTyping",
    "placeholder",
  ]);
});

test("feature slots can omit or replace a chip module", () => {
  const omitted = createComposerExtensions({
    features: { link: false, fileChip: false },
  }).map((extension) => extension.name);
  assert.equal(omitted.includes("link"), false);
  assert.equal(omitted.includes("composerFile"), false);
  assert.equal(omitted.includes("promptToken"), true);
});

test("composer heading input covers Markdown levels 1 through 6", () => {
  assert.deepEqual([...COMPOSER_HEADING_LEVELS], [1, 2, 3, 4, 5, 6]);
});

test("fence openers keep C++ and similar language ids until Shift+Enter or space", () => {
  assert.deepEqual(parseFenceOpener("```"), { language: null });
  assert.deepEqual(parseFenceOpener("```C++"), { language: "C++" });
  assert.deepEqual(parseFenceOpener("```c#"), { language: "c#" });
  assert.deepEqual(parseFenceOpener("```objective-c"), {
    language: "objective-c",
  });
  assert.equal(parseFenceOpener("```ts code"), null);
});

test("plain text round-trips through composer JSON without HTML parsing", () => {
  const content = plainTextToComposerContent("first\n\nsecond <script>");
  assert.deepEqual(content, {
    type: "doc",
    content: [
      { type: "paragraph", content: [{ type: "text", text: "first" }] },
      { type: "paragraph" },
      {
        type: "paragraph",
        content: [{ type: "text", text: "second <script>" }],
      },
    ],
  });
});

test("file chips serialize to backtick path:line payloads", () => {
  assert.equal(
    composerFilePlainText({
      path: "src/app.ts",
      startLine: 4,
      endLine: 12,
    }),
    "`src/app.ts:4-12`",
  );
  assert.equal(
    composerFilePlainText({ path: "README.md", startLine: 3, endLine: 3 }),
    "`README.md:3`",
  );
});

test("documentPlainText serializes prompt token chips back to $ / prefixes", () => {
  const schema = new Schema({
    nodes: {
      doc: { content: "block+" },
      paragraph: { content: "inline*", group: "block" },
      text: { group: "inline" },
      promptToken: {
        group: "inline",
        inline: true,
        atom: true,
        attrs: {
          kind: { default: "skill" },
          name: { default: "" },
        },
      },
    },
  });
  const doc = schema.node("doc", null, [
    schema.node("paragraph", null, [
      schema.node("promptToken", { kind: "skill", name: "code-review" }),
      schema.text(" "),
    ]),
  ]);
  assert.equal(documentPlainText(doc), "$code-review ");
});

test("documentPlainText inserts spaces between adjacent chips without doc spaces", () => {
  const schema = new Schema({
    nodes: {
      doc: { content: "block+" },
      paragraph: { content: "inline*", group: "block" },
      text: { group: "inline" },
      promptToken: {
        group: "inline",
        inline: true,
        atom: true,
        attrs: {
          kind: { default: "skill" },
          name: { default: "" },
        },
      },
      composerFile: {
        group: "inline",
        inline: true,
        atom: true,
        attrs: {
          path: { default: "" },
          startLine: { default: null },
          endLine: { default: null },
        },
      },
    },
  });
  const doc = schema.node("doc", null, [
    schema.node("paragraph", null, [
      schema.node("promptToken", { kind: "skill", name: "dev-expert" }),
      schema.node("composerFile", { path: ".codex" }),
      schema.node("composerFile", { path: "hack.svg" }),
      schema.text(" notes"),
    ]),
  ]);
  assert.equal(documentPlainText(doc), "$dev-expert `.codex` `hack.svg` notes");
});

test("documentPlainText serializes markdown links and file chips for the agent payload", () => {
  const schema = new Schema({
    nodes: {
      doc: { content: "block+" },
      paragraph: { content: "inline*", group: "block" },
      text: { group: "inline" },
      composerFile: {
        group: "inline",
        inline: true,
        atom: true,
        attrs: {
          path: { default: "" },
          startLine: { default: null },
          endLine: { default: null },
        },
      },
    },
    marks: {
      link: {
        attrs: { href: { default: null }, title: { default: null } },
        inclusive: false,
        parseDOM: [{ tag: "a[href]" }],
        toDOM(mark) {
          return ["a", { href: mark.attrs.href, title: mark.attrs.title }, 0];
        },
      },
    },
  });
  const doc = schema.node("doc", null, [
    schema.node("paragraph", null, [
      schema.text("haha", [
        schema.mark("link", { href: "http://www.baidu.com" }),
      ]),
      schema.text(" "),
      schema.node("composerFile", {
        path: "src/a.ts",
        startLine: 1,
        endLine: 2,
      }),
      schema.text(" "),
      schema.text("Docs", [
        schema.mark("link", {
          href: "https://example.com",
          title: "hover",
        }),
      ]),
    ]),
  ]);
  assert.equal(
    documentPlainText(doc),
    '[haha](http://www.baidu.com) `src/a.ts:1-2` [Docs](https://example.com "hover")',
  );
});

test("documentPlainText joins blocks and hard breaks with a single newline", () => {
  const schema = new Schema({
    nodes: {
      doc: { content: "block+" },
      paragraph: { content: "inline*", group: "block" },
      text: { group: "inline" },
      hardBreak: { group: "inline", inline: true, selectable: false },
    },
  });
  const doc = schema.node("doc", null, [
    schema.node("paragraph", null, [
      schema.text("first"),
      schema.node("hardBreak"),
      schema.text("second"),
    ]),
  ]);
  assert.equal(documentPlainText(doc), "first\nsecond");
});

test("documentPlainText serializes horizontal rules as markdown dashes", () => {
  const schema = new Schema({
    nodes: {
      doc: { content: "block+" },
      paragraph: { content: "inline*", group: "block" },
      text: { group: "inline" },
      horizontalRule: { group: "block" },
    },
  });
  const doc = schema.node("doc", null, [
    schema.node("paragraph", null, [schema.text("above")]),
    schema.node("horizontalRule"),
    schema.node("paragraph", null, [schema.text("below")]),
  ]);
  assert.equal(documentPlainText(doc), "above\n---\nbelow");
});

test("documentPlainText serializes headings, lists, quotes, fences, and marks", () => {
  const schema = new Schema({
    nodes: {
      doc: { content: "block+" },
      paragraph: { content: "inline*", group: "block" },
      text: { group: "inline" },
      heading: {
        content: "inline*",
        group: "block",
        attrs: { level: { default: 1 } },
      },
      codeBlock: {
        content: "text*",
        group: "block",
        code: true,
        attrs: { language: { default: null } },
      },
      blockquote: { content: "block+", group: "block" },
      bulletList: { content: "listItem+", group: "block" },
      listItem: { content: "paragraph block*", defining: true },
    },
    marks: {
      bold: {},
      italic: {},
      strike: {},
      code: {},
      highlight: {},
    },
  });
  const doc = schema.node("doc", null, [
    schema.node("heading", { level: 1 }, [schema.text("Title")]),
    schema.node("heading", { level: 6 }, [schema.text("Fine")]),
    schema.node("blockquote", null, [
      schema.node("paragraph", null, [schema.text("quoted")]),
    ]),
    schema.node("bulletList", null, [
      schema.node("listItem", null, [
        schema.node("paragraph", null, [schema.text("item")]),
      ]),
    ]),
    schema.node("codeBlock", { language: "ts" }, [schema.text("const n = 1;")]),
    schema.node("paragraph", null, [
      schema.text("bold", [schema.mark("bold")]),
      schema.text(" "),
      schema.text("hi", [schema.mark("highlight")]),
    ]),
  ]);
  assert.equal(
    documentPlainText(doc),
    "# Title\n###### Fine\n> quoted\n- item\n```ts\nconst n = 1;\n```\n**bold** ==hi==",
  );
});

test("highlightInputMatch keeps only the inner text so == is not stored", () => {
  assert.deepEqual(highlightInputMatch("==hi=="), {
    index: 0,
    text: "==hi==",
    replaceWith: "hi",
  });
  assert.deepEqual(highlightInputMatch("==高亮=="), {
    index: 0,
    text: "==高亮==",
    replaceWith: "高亮",
  });
  assert.deepEqual(highlightInputMatch("- ==高亮=="), {
    index: 2,
    text: "==高亮==",
    replaceWith: "高亮",
  });
  assert.equal(highlightInputMatch("== d =="), null);
});

test("boldItalicInputMatch keeps only the inner text of ***both***", () => {
  assert.deepEqual(boldItalicInputMatch("***both***"), {
    index: 0,
    text: "***both***",
    replaceWith: "both",
  });
  assert.deepEqual(boldItalicInputMatch("***粗斜体***"), {
    index: 0,
    text: "***粗斜体***",
    replaceWith: "粗斜体",
  });
  assert.equal(boldItalicInputMatch("**bold**"), null);
});

test("isComposerOpenableUrl matches Desktop open_external schemes", () => {
  assert.equal(isComposerOpenableUrl("https://example.com/path"), true);
  assert.equal(isComposerOpenableUrl("http://example.com"), true);
  assert.equal(isComposerOpenableUrl("mailto:dev@example.com"), true);
  assert.equal(isComposerOpenableUrl("HTTPS://EXAMPLE.COM"), true);
  assert.equal(isComposerOpenableUrl("tel:+123"), false);
  assert.equal(isComposerOpenableUrl("ftp://files.example.com"), false);
  assert.equal(isComposerOpenableUrl("javascript:alert(1)"), false);
  assert.equal(
    isComposerOpenableUrl("https://example.com/path with space"),
    false,
  );
  assert.equal(isComposerOpenableUrl(""), false);
});

test("range selection decorates intersecting chips for visual highlight", async () => {
  const { Editor } = await import("@tiptap/core");
  const { chipSelectionDecorations } =
    await import("../src/composer/composer-chip-selection.ts");
  const editor = new Editor({
    extensions: createComposerExtensions({ placeholder: "Type" }),
    content: {
      type: "doc",
      content: [
        {
          type: "paragraph",
          content: [
            { type: "text", text: "pre" },
            {
              type: "composerFile",
              attrs: { path: "AGENTS.md", kind: "file" },
            },
            { type: "text", text: "post" },
          ],
        },
      ],
    },
  });
  const size = editor.state.doc.content.size;
  editor.commands.setTextSelection({ from: 1, to: size - 1 });
  const decorated = chipSelectionDecorations(editor.state);
  // ProseMirror keeps attrs on the internal decoration type; narrow for the assert.
  const classes = decorated
    .find()
    .map((decoration) => {
      const attrs = (
        decoration as unknown as { type: { attrs?: { class?: string } } }
      ).type.attrs;
      return attrs?.class;
    })
    .filter(Boolean);
  assert.equal(classes.includes("composer-chip-in-selection"), true);
  editor.destroy();
});

test("documentPlainText uses a longer fence when the code block contains ```", () => {
  const schema = new Schema({
    nodes: {
      doc: { content: "block+" },
      paragraph: { content: "inline*", group: "block" },
      text: { group: "inline" },
      codeBlock: {
        content: "text*",
        group: "block",
        code: true,
        attrs: { language: { default: null } },
      },
    },
  });
  const doc = schema.node("doc", null, [
    schema.node("codeBlock", { language: null }, [schema.text("```\ncode")]),
  ]);
  assert.equal(documentPlainText(doc), "````\n```\ncode\n````");
});

test("documentPlainText wraps inline code that contains backticks", () => {
  const schema = new Schema({
    nodes: {
      doc: { content: "block+" },
      paragraph: { content: "inline*", group: "block" },
      text: { group: "inline" },
    },
    marks: { code: {} },
  });
  const doc = schema.node("doc", null, [
    schema.node("paragraph", null, [schema.text("a`b", [schema.mark("code")])]),
  ]);
  assert.equal(documentPlainText(doc), "`` a`b ``");
});

test("documentPlainText escapes backslashes before quotes in link titles", () => {
  const schema = new Schema({
    nodes: {
      doc: { content: "block+" },
      paragraph: { content: "inline*", group: "block" },
      text: { group: "inline" },
    },
    marks: {
      link: {
        attrs: { href: { default: null }, title: { default: null } },
        inclusive: false,
      },
    },
  });
  const doc = schema.node("doc", null, [
    schema.node("paragraph", null, [
      schema.text("Docs", [
        schema.mark("link", {
          href: "https://example.com",
          title: 'say "hi"',
        }),
      ]),
    ]),
  ]);
  assert.equal(
    documentPlainText(doc),
    '[Docs](https://example.com "say \\"hi\\"")',
  );
});

test("composerFileLabel uses the last path segment even when the path ends with a slash", () => {
  assert.equal(
    composerFileLabel({ path: "foo/bar/", kind: "directory" }),
    "bar",
  );
});
