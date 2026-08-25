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
  composerFileAttrsFromUnknown,
  composerFileChipTitle,
  composerFileLabel,
  composerFilePlainText,
} from "../src/composer/composer-file.ts";
import { parseComposerFileQuote } from "../src/composer/composer-file-quote.ts";
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

test("composerFileChipTitle keeps one-line path:range and uses path for multiline payloads", () => {
  assert.equal(
    composerFileChipTitle({
      path: "src/app.ts",
      startLine: 4,
      endLine: 12,
    }),
    "src/app.ts:4-12",
  );
  assert.equal(
    composerFileChipTitle({
      path: "src/app.ts",
      startLine: 4,
      endLine: 5,
      snippet: "const a = 1;\nconst b = 2;",
    }),
    "src/app.ts",
  );
});

test("composerFileAttrsFromUnknown drops non-positive and NaN line numbers", () => {
  assert.deepEqual(
    composerFileAttrsFromUnknown({
      path: "a.ts",
      startLine: Number.NaN,
      endLine: 0,
      snippet: null,
      origin: "diff",
      diffSide: "new",
    }),
    {
      path: "a.ts",
      startLine: undefined,
      endLine: undefined,
      snippet: undefined,
      kind: "file",
      origin: "diff",
      diffSide: "new",
    },
  );
});

test("file chips with snippets serialize to citation fences", () => {
  assert.equal(
    composerFilePlainText({
      path: "src/app.ts",
      startLine: 4,
      endLine: 5,
      snippet: "const a = 1;\nconst b = 2;",
    }),
    "\n```4:5:src/app.ts\nconst a = 1;\nconst b = 2;\n```\n",
  );
});

test("diff-gutter quotes serialize as a git patch so the agent sees an existing change", () => {
  assert.equal(
    composerFilePlainText({
      path: "src/example.ts",
      startLine: 1,
      endLine: 2,
      snippet: " keep\n+new line",
      origin: "diff",
      diffSide: "new",
    }),
    [
      "",
      "```diff",
      "diff --git a/src/example.ts b/src/example.ts",
      "--- a/src/example.ts",
      "+++ b/src/example.ts",
      "@@ -1,1 +1,2 @@ quoted from git diff (new side), lines 1-2",
      " keep",
      "+new line",
      "```",
      "",
    ].join("\n"),
  );
  assert.equal(
    composerFilePlainText({
      path: "src/example.ts",
      startLine: 2,
      endLine: 2,
      snippet: "-old line",
      origin: "diff",
      diffSide: "old",
    }),
    [
      "",
      "```diff",
      "diff --git a/src/example.ts b/src/example.ts",
      "--- a/src/example.ts",
      "+++ b/src/example.ts",
      "@@ -2,1 +2,0 @@ quoted from git diff (old side), lines 2-2",
      "-old line",
      "```",
      "",
    ].join("\n"),
  );
  assert.equal(
    composerFilePlainText({
      path: "src/example.ts",
      startLine: 1,
      endLine: 3,
      snippet: " keep\n-old line\n+new line",
      origin: "diff",
    }),
    [
      "",
      "```diff",
      "diff --git a/src/example.ts b/src/example.ts",
      "--- a/src/example.ts",
      "+++ b/src/example.ts",
      "@@ -1,2 +1,2 @@ quoted from git diff, lines 1-3",
      " keep",
      "-old line",
      "+new line",
      "```",
      "",
    ].join("\n"),
  );
});

/** Splits a serialized quote back into the fence info string and its body. */
function fencedQuoteParts(payload: string): { info: string; body: string } {
  const lines = payload.replace(/^\n/, "").replace(/\n$/, "").split("\n");
  const opener = lines[0] ?? "";
  return {
    info: opener.replace(/^`+/, ""),
    body: lines.slice(1, -1).join("\n"),
  };
}

test("parseComposerFileQuote round-trips every payload composerFilePlainText writes", () => {
  const quotes = [
    {
      path: "src/app.ts",
      startLine: 4,
      endLine: 5,
      snippet: "const a = 1;\nconst b = 2;",
      kind: "file" as const,
      origin: undefined,
      diffSide: undefined,
    },
    {
      path: "src/example.ts",
      startLine: 1,
      endLine: 2,
      snippet: " keep\n+new line",
      kind: "file" as const,
      origin: "diff" as const,
      diffSide: "new" as const,
    },
    {
      path: "src/example.ts",
      startLine: 2,
      endLine: 2,
      snippet: "-old line",
      kind: "file" as const,
      origin: "diff" as const,
      diffSide: "old" as const,
    },
    {
      // A drag across a collapsed hunk quotes a wider span than the body has
      // lines, so only the range note can carry the label history rebuilds.
      path: "src/example.ts",
      startLine: 2,
      endLine: 40,
      snippet: " keep\n-old line\n+new line",
      kind: "file" as const,
      origin: "diff" as const,
      diffSide: undefined,
    },
  ];

  for (const quote of quotes) {
    const { info, body } = fencedQuoteParts(composerFilePlainText(quote));
    assert.deepEqual(parseComposerFileQuote(info, body), quote);
  }
});

test("parseComposerFileQuote leaves ordinary fenced code alone", () => {
  assert.equal(parseComposerFileQuote("ts", "const a = 1;"), null);
  assert.equal(parseComposerFileQuote("", "plain text"), null);
  // Renamed file: a quote always patches one path against itself.
  assert.equal(
    parseComposerFileQuote("diff", "diff --git a/a.ts b/b.ts\n+added"),
    null,
  );
  // A hand-written patch without the quote note stays a diff code block.
  assert.equal(
    parseComposerFileQuote(
      "diff",
      [
        "diff --git a/a.ts b/a.ts",
        "--- a/a.ts",
        "+++ b/a.ts",
        "@@ -1,1 +1,1 @@",
        "+added",
      ].join("\n"),
    ),
    null,
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

test("pointer over a chip includes the whole atom before the midpoint", async () => {
  const { Editor } = await import("@tiptap/core");
  const { textSelectionForChipDrag } =
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
  let chipPos = -1;
  editor.state.doc.descendants((node, pos) => {
    if (node.type.name === "composerFile") {
      chipPos = pos;
      return false;
    }
    return true;
  });
  const chip = editor.state.doc.nodeAt(chipPos);
  assert.ok(chip);
  // Caret maps to the left edge (chipPos) while the pointer is already on the chip.
  const forward = textSelectionForChipDrag(
    editor.state.doc,
    1,
    chipPos,
    chipPos,
  );
  assert.equal(forward.from, 1);
  assert.equal(forward.to, chipPos + chip.nodeSize);

  const backward = textSelectionForChipDrag(
    editor.state.doc,
    chipPos + chip.nodeSize + 2,
    chipPos + chip.nodeSize,
    chipPos,
  );
  assert.equal(backward.from, chipPos);
  assert.equal(backward.to, chipPos + chip.nodeSize + 2);

  // Press on the right half (caret after the atom) then drag left across it.
  const fromRightHalf = textSelectionForChipDrag(
    editor.state.doc,
    chipPos + chip.nodeSize,
    chipPos,
    chipPos,
  );
  assert.equal(fromRightHalf.from, chipPos);
  assert.equal(fromRightHalf.to, chipPos + chip.nodeSize);

  // Dragging left off the chip's start must not keep the chip selected.
  const away = textSelectionForChipDrag(editor.state.doc, chipPos, 1, -1);
  assert.equal(away.from, 1);
  assert.equal(away.to, chipPos);
  editor.destroy();
});

test("node selection can target a single file chip among adjacent siblings", async () => {
  const { Editor } = await import("@tiptap/core");
  const { NodeSelection } = await import("@tiptap/pm/state");
  const editor = new Editor({
    extensions: createComposerExtensions({ placeholder: "Type" }),
    content: {
      type: "doc",
      content: [
        {
          type: "paragraph",
          content: [
            {
              type: "composerFile",
              attrs: { path: "a.ts", startLine: 1, endLine: 1, kind: "file" },
            },
            {
              type: "composerFile",
              attrs: { path: "b.ts", startLine: 2, endLine: 2, kind: "file" },
            },
            {
              type: "composerFile",
              attrs: { path: "c.ts", startLine: 3, endLine: 3, kind: "file" },
            },
            { type: "text", text: " tail" },
          ],
        },
      ],
    },
  });

  let secondChipPos: number | null = null;
  editor.state.doc.descendants((node, pos) => {
    if (node.type.name === "composerFile" && node.attrs.path === "b.ts") {
      secondChipPos = pos;
      return false;
    }
    return true;
  });
  assert.ok(secondChipPos !== null);
  editor.commands.setNodeSelection(secondChipPos!);
  assert.ok(editor.state.selection instanceof NodeSelection);
  const selected = editor.state.selection as InstanceType<typeof NodeSelection>;
  assert.equal(selected.node.attrs.path, "b.ts");
  assert.equal(editor.state.selection.from, secondChipPos);
  assert.equal(
    editor.state.selection.to,
    secondChipPos! + selected.node.nodeSize,
  );
  editor.destroy();
});

test("pinComposerChipSelection keeps NodeSelection on one adjacent chip", async () => {
  const { Editor } = await import("@tiptap/core");
  const { NodeSelection } = await import("@tiptap/pm/state");
  const { pinComposerChipSelection } =
    await import("../src/composer/composer-chip-selection.ts");
  const editor = new Editor({
    extensions: createComposerExtensions({ placeholder: "Type" }),
    content: {
      type: "doc",
      content: [
        {
          type: "paragraph",
          content: [
            {
              type: "composerFile",
              attrs: { path: "a.ts", startLine: 1, endLine: 1, kind: "file" },
            },
            {
              type: "composerFile",
              attrs: { path: "b.ts", startLine: 2, endLine: 2, kind: "file" },
            },
            {
              type: "composerFile",
              attrs: { path: "c.ts", startLine: 3, endLine: 3, kind: "file" },
            },
          ],
        },
      ],
    },
  });

  let secondChipPos = -1;
  editor.state.doc.descendants((node, pos) => {
    if (node.type.name === "composerFile" && node.attrs.path === "b.ts") {
      secondChipPos = pos;
      return false;
    }
    return true;
  });
  assert.ok(secondChipPos >= 0);

  let prevented = false;
  // Headless TipTap still applies transactions through commands; mirror the
  // plugin's dispatch path with a minimal view facade over editor state.
  let state = editor.state;
  const view = {
    get state() {
      return state;
    },
    dispatch(tr: Parameters<typeof state.apply>[0]) {
      state = state.apply(tr);
    },
  };
  assert.equal(
    pinComposerChipSelection(view, secondChipPos, {
      preventDefault() {
        prevented = true;
      },
    }),
    true,
  );
  assert.equal(prevented, true);
  assert.ok(state.selection instanceof NodeSelection);
  assert.equal(
    (state.selection as InstanceType<typeof NodeSelection>).node.attrs.path,
    "b.ts",
  );
  assert.equal(state.selection.from, secondChipPos);
  assert.equal(
    state.selection.to,
    secondChipPos +
      (state.selection as InstanceType<typeof NodeSelection>).node.nodeSize,
  );
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
