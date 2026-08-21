import assert from "node:assert/strict";
import { describe, expect, it } from "vitest";
import {
  prepareUserMessageMarkdown,
  remarkComposerHighlight,
} from "./user-message-markdown";

describe("prepareUserMessageMarkdown", () => {
  it("expands single newlines outside fences into paragraph breaks", () => {
    expect(prepareUserMessageMarkdown("hello\nworld")).toBe("hello\n\nworld");
    expect(prepareUserMessageMarkdown("# Title\nbody")).toBe("# Title\n\nbody");
  });

  it("leaves blank lines and fence interiors alone", () => {
    expect(prepareUserMessageMarkdown("a\n\nb")).toBe("a\n\nb");
    expect(prepareUserMessageMarkdown("```ts\nline1\nline2\n```")).toBe(
      "```ts\nline1\nline2\n```",
    );
    expect(
      prepareUserMessageMarkdown("before\n```\nkeep\nme\n```\nafter\nnext"),
    ).toBe("before\n```\nkeep\nme\n```\nafter\n\nnext");
  });

  it("expands every single-character line, not only the first break", () => {
    expect(prepareUserMessageMarkdown("a\nb\nc")).toBe("a\n\nb\n\nc");
  });
});

describe("remarkComposerHighlight", () => {
  it("splits ==highlight== text into mark hast mapping", () => {
    const tree = {
      type: "root",
      children: [
        {
          type: "paragraph",
          children: [{ type: "text", value: "x ==hi== y" }],
        },
      ],
    };
    remarkComposerHighlight()(tree);
    const paragraph = tree.children[0];
    assert.ok(paragraph);
    expect(paragraph.children).toEqual([
      { type: "text", value: "x " },
      {
        type: "emphasis",
        data: {
          hName: "mark",
          hProperties: { className: "composer-user-highlight" },
        },
        children: [{ type: "text", value: "hi" }],
      },
      { type: "text", value: " y" },
    ]);
  });

  it("does not rewrite == inside code nodes", () => {
    const tree = {
      type: "root",
      children: [
        { type: "code", value: "==raw==" },
        {
          type: "paragraph",
          children: [{ type: "inlineCode", value: "==raw==" }],
        },
      ],
    };
    remarkComposerHighlight()(tree);
    expect(tree.children[0]).toEqual({ type: "code", value: "==raw==" });
    expect(tree.children[1]?.children).toEqual([
      { type: "inlineCode", value: "==raw==" },
    ]);
  });
});
