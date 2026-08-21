import assert from "node:assert/strict";
import test from "node:test";
import {
  looksLikeComposerMarkdown,
  markdownToComposerContent,
} from "../src/composer/composer-markdown.ts";

test("looksLikeComposerMarkdown ignores ordinary sentences", () => {
  assert.equal(looksLikeComposerMarkdown("hello world"), false);
  assert.equal(looksLikeComposerMarkdown("a * b = c"), false);
  assert.equal(looksLikeComposerMarkdown("2 * 3 * 4"), false);
  assert.equal(looksLikeComposerMarkdown(""), false);
});

test("markdownToComposerContent covers the prompt-box Markdown surface", () => {
  const doc = markdownToComposerContent(
    [
      "# Title",
      "###### Fine",
      "",
      "> quoted **bold**",
      "",
      "- bullet",
      "1. numbered",
      "- [ ] todo",
      "- [x] done",
      "",
      "---",
      "",
      "```ts",
      "const n = 1;",
      "```",
      "",
      "**bold** *em* ~~out~~ `code` ==hi== [Docs](https://example.com)",
    ].join("\n"),
  );

  assert.deepEqual(doc, {
    type: "doc",
    content: [
      {
        type: "heading",
        attrs: { level: 1 },
        content: [{ type: "text", text: "Title" }],
      },
      {
        type: "heading",
        attrs: { level: 6 },
        content: [{ type: "text", text: "Fine" }],
      },
      {
        type: "blockquote",
        content: [
          {
            type: "paragraph",
            content: [
              { type: "text", text: "quoted " },
              { type: "text", text: "bold", marks: [{ type: "bold" }] },
            ],
          },
        ],
      },
      {
        type: "bulletList",
        content: [
          {
            type: "listItem",
            content: [
              {
                type: "paragraph",
                content: [{ type: "text", text: "bullet" }],
              },
            ],
          },
        ],
      },
      {
        type: "orderedList",
        content: [
          {
            type: "listItem",
            content: [
              {
                type: "paragraph",
                content: [{ type: "text", text: "numbered" }],
              },
            ],
          },
        ],
      },
      {
        type: "taskList",
        content: [
          {
            type: "taskItem",
            attrs: { checked: false },
            content: [
              {
                type: "paragraph",
                content: [{ type: "text", text: "todo" }],
              },
            ],
          },
          {
            type: "taskItem",
            attrs: { checked: true },
            content: [
              {
                type: "paragraph",
                content: [{ type: "text", text: "done" }],
              },
            ],
          },
        ],
      },
      { type: "horizontalRule" },
      {
        type: "codeBlock",
        attrs: { language: "ts" },
        content: [{ type: "text", text: "const n = 1;" }],
      },
      {
        type: "paragraph",
        content: [
          { type: "text", text: "bold", marks: [{ type: "bold" }] },
          { type: "text", text: " " },
          { type: "text", text: "em", marks: [{ type: "italic" }] },
          { type: "text", text: " " },
          { type: "text", text: "out", marks: [{ type: "strike" }] },
          { type: "text", text: " " },
          { type: "text", text: "code", marks: [{ type: "code" }] },
          { type: "text", text: " " },
          { type: "text", text: "hi", marks: [{ type: "highlight" }] },
          { type: "text", text: " " },
          {
            type: "text",
            text: "Docs",
            marks: [{ type: "link", attrs: { href: "https://example.com" } }],
          },
        ],
      },
    ],
  });
});

test("markdownToComposerContent keeps HTML tags as text", () => {
  const doc = markdownToComposerContent("<script>alert(1)</script>");
  assert.deepEqual(doc, {
    type: "doc",
    content: [
      {
        type: "paragraph",
        content: [{ type: "text", text: "<script>alert(1)</script>" }],
      },
    ],
  });
});

test("markdownToComposerContent accepts plus bullets, star tasks, and mid-word strike", () => {
  const doc = markdownToComposerContent(
    ["+ plus", "* [ ] star", "sa~~d~~ __bold__ _em_"].join("\n"),
  );
  assert.deepEqual(doc, {
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
                content: [{ type: "text", text: "plus" }],
              },
            ],
          },
        ],
      },
      {
        type: "taskList",
        content: [
          {
            type: "taskItem",
            attrs: { checked: false },
            content: [
              {
                type: "paragraph",
                content: [{ type: "text", text: "star" }],
              },
            ],
          },
        ],
      },
      {
        type: "paragraph",
        content: [
          { type: "text", text: "sa" },
          { type: "text", text: "d", marks: [{ type: "strike" }] },
          { type: "text", text: " " },
          { type: "text", text: "bold", marks: [{ type: "bold" }] },
          { type: "text", text: " " },
          { type: "text", text: "em", marks: [{ type: "italic" }] },
        ],
      },
    ],
  });
});

test("markdownToComposerContent follows GFM flanking for marks", () => {
  assert.deepEqual(markdownToComposerContent("你好**等等**"), {
    type: "doc",
    content: [
      {
        type: "paragraph",
        content: [
          { type: "text", text: "你好" },
          { type: "text", text: "等等", marks: [{ type: "bold" }] },
        ],
      },
    ],
  });
  assert.deepEqual(paragraphPlain(markdownToComposerContent("2 * 3 * 4")), {
    text: "2 * 3 * 4",
    marks: [],
  });
  assert.deepEqual(paragraphPlain(markdownToComposerContent("ddssd**d **")), {
    text: "ddssd**d **",
    marks: [],
  });
  assert.deepEqual(paragraphPlain(markdownToComposerContent("foo_bar_baz")), {
    text: "foo_bar_baz",
    marks: [],
  });
});

test("markdownToComposerContent nests quotes, lists in quotes, titled links, and ***", () => {
  const doc = markdownToComposerContent(
    [
      "> outer",
      "> > inner",
      "> - listed",
      "",
      '***both*** [Docs](https://example.com "hover")',
    ].join("\n"),
  );
  assert.deepEqual(doc, {
    type: "doc",
    content: [
      {
        type: "blockquote",
        content: [
          {
            type: "paragraph",
            content: [{ type: "text", text: "outer" }],
          },
          {
            type: "blockquote",
            content: [
              {
                type: "paragraph",
                content: [{ type: "text", text: "inner" }],
              },
            ],
          },
          {
            type: "bulletList",
            content: [
              {
                type: "listItem",
                content: [
                  {
                    type: "paragraph",
                    content: [{ type: "text", text: "listed" }],
                  },
                ],
              },
            ],
          },
        ],
      },
      {
        type: "paragraph",
        content: [
          {
            type: "text",
            text: "both",
            marks: [{ type: "italic" }, { type: "bold" }],
          },
          { type: "text", text: " " },
          {
            type: "text",
            text: "Docs",
            marks: [
              {
                type: "link",
                attrs: { href: "https://example.com", title: "hover" },
              },
            ],
          },
        ],
      },
    ],
  });
  assert.deepEqual(markdownToComposerContent("***"), {
    type: "doc",
    content: [{ type: "horizontalRule" }],
  });
  assert.deepEqual(
    paragraphPlain(
      markdownToComposerContent('![alt](https://example.com "t")'),
    ),
    {
      text: '![alt](https://example.com "t")',
      marks: [],
    },
  );
});

test("markdownToComposerContent shares a middle *** between adjacent marks", () => {
  assert.deepEqual(markdownToComposerContent("**加粗***倾斜*"), {
    type: "doc",
    content: [
      {
        type: "paragraph",
        content: [
          { type: "text", text: "加粗", marks: [{ type: "bold" }] },
          { type: "text", text: "倾斜", marks: [{ type: "italic" }] },
        ],
      },
    ],
  });
  assert.deepEqual(markdownToComposerContent("*倾斜***加粗**"), {
    type: "doc",
    content: [
      {
        type: "paragraph",
        content: [
          { type: "text", text: "倾斜", marks: [{ type: "italic" }] },
          { type: "text", text: "加粗", marks: [{ type: "bold" }] },
        ],
      },
    ],
  });
  assert.deepEqual(markdownToComposerContent("**bold** "), {
    type: "doc",
    content: [
      {
        type: "paragraph",
        content: [
          { type: "text", text: "bold", marks: [{ type: "bold" }] },
          { type: "text", text: " " },
        ],
      },
    ],
  });
});

test("markdownToComposerContent keeps trailing space after every inline wrap", () => {
  assert.deepEqual(paragraphPlain(markdownToComposerContent("==a== ")), {
    text: "a ",
    marks: ["highlight"],
  });
  assert.deepEqual(paragraphPlain(markdownToComposerContent("~~a~~ ")), {
    text: "a ",
    marks: ["strike"],
  });
  assert.deepEqual(paragraphPlain(markdownToComposerContent("*a* ")), {
    text: "a ",
    marks: ["italic"],
  });
  assert.deepEqual(paragraphPlain(markdownToComposerContent("`a` ")), {
    text: "a ",
    marks: ["code"],
  });
  assert.deepEqual(paragraphPlain(markdownToComposerContent("***a*** ")), {
    text: "a ",
    marks: ["italic", "bold"],
  });
  assert.deepEqual(paragraphPlain(markdownToComposerContent("__a__ ")), {
    text: "a ",
    marks: ["bold"],
  });
  assert.deepEqual(paragraphPlain(markdownToComposerContent("_a_ ")), {
    text: "a ",
    marks: ["italic"],
  });
  assert.deepEqual(
    paragraphPlain(markdownToComposerContent("[Docs](https://example.com) ")),
    {
      text: "Docs ",
      marks: ["link"],
    },
  );
});

test("markdownToComposerContent caps quote nesting so a deep paste cannot recurse forever", () => {
  const doc = markdownToComposerContent(`${"> ".repeat(80)}deep`);
  let depth = 0;
  let node: { type?: string; content?: unknown } | undefined = doc.content?.[0];
  while (node?.type === "blockquote") {
    depth += 1;
    const inner = node.content;
    node = Array.isArray(inner) ? inner[0] : undefined;
  }
  assert.equal(depth, 32);
  assert.equal(typeof node?.type, "string");
});

test("markdownToComposerContent restores path backticks and $skills as chips", () => {
  const doc = markdownToComposerContent(
    "see `src/app.ts:1-2` and $code-review then /test",
  );
  assert.deepEqual(doc, {
    type: "doc",
    content: [
      {
        type: "paragraph",
        content: [
          { type: "text", text: "see " },
          {
            type: "composerFile",
            attrs: {
              path: "src/app.ts",
              startLine: 1,
              endLine: 2,
              kind: "file",
            },
          },
          { type: "text", text: " and " },
          {
            type: "promptToken",
            attrs: { kind: "skill", name: "code-review" },
          },
          { type: "text", text: " then /test" },
        ],
      },
    ],
  });
});

test("markdownToComposerContent keeps plain backticks as inline code", () => {
  const doc = markdownToComposerContent("use `code` not a path");
  assert.deepEqual(doc.content?.[0], {
    type: "paragraph",
    content: [
      { type: "text", text: "use " },
      { type: "text", text: "code", marks: [{ type: "code" }] },
      { type: "text", text: " not a path" },
    ],
  });
});

test("markdownToComposerContent does not chip versions or globs", () => {
  const doc = markdownToComposerContent("pin `v1.0` and `*.ts` please");
  assert.deepEqual(doc.content?.[0], {
    type: "paragraph",
    content: [
      { type: "text", text: "pin " },
      { type: "text", text: "v1.0", marks: [{ type: "code" }] },
      { type: "text", text: " and " },
      { type: "text", text: "*.ts", marks: [{ type: "code" }] },
      { type: "text", text: " please" },
    ],
  });
});

test("markdownToComposerContent keeps backticks inside longer inline-code fences", () => {
  assert.deepEqual(paragraphPlain(markdownToComposerContent("`` a`b ``")), {
    text: "a`b",
    marks: ["code"],
  });
});

test("markdownToComposerContent does not close a fence on an inner ``` line", () => {
  const doc = markdownToComposerContent("````\n```\ncode\n````");
  assert.deepEqual(doc, {
    type: "doc",
    content: [
      {
        type: "codeBlock",
        attrs: { language: null },
        content: [{ type: "text", text: "```\ncode" }],
      },
    ],
  });
});

test("markdownToComposerContent keeps escaped quotes in link titles", () => {
  const doc = markdownToComposerContent(
    '[Docs](https://example.com "say \\"hi\\"")',
  );
  assert.deepEqual(doc.content?.[0], {
    type: "paragraph",
    content: [
      {
        type: "text",
        text: "Docs",
        marks: [
          {
            type: "link",
            attrs: { href: "https://example.com", title: 'say "hi"' },
          },
        ],
      },
    ],
  });
});

test("markdownToComposerContent keeps javascript hrefs as literal text", () => {
  const doc = markdownToComposerContent("[xss](javascript:alert(1))");
  assert.deepEqual(doc, {
    type: "doc",
    content: [
      {
        type: "paragraph",
        content: [{ type: "text", text: "[xss](javascript:alert(1))" }],
      },
    ],
  });
});

function paragraphPlain(doc: {
  content?: Array<{
    content?: Array<{ text?: string; marks?: Array<{ type: string }> }>;
  }>;
}): { text: string; marks: string[] } {
  const nodes = doc.content?.[0]?.content ?? [];
  return {
    text: nodes.map((node) => node.text ?? "").join(""),
    marks: nodes.flatMap((node) => (node.marks ?? []).map((mark) => mark.type)),
  };
}
