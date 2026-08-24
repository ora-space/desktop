import { describe, expect, it, vi } from "vitest";
import type { ChatToolCall, ChatTurn } from "@ora/chat";
import {
  collectCumulativeArtifactIndices,
  collectSessionArtifactIndex,
} from "./artifact-index";
import * as turnDiffFiles from "../turn-diff-files";

/** Builds a tool call without involving the ACP transport. */
function tool(
  partial: Partial<ChatToolCall> & Pick<ChatToolCall, "id">,
): ChatToolCall {
  return {
    title: partial.title ?? partial.id,
    toolKind: partial.toolKind,
    status: partial.status ?? "completed",
    content: partial.content ?? [],
    locations: partial.locations ?? [],
    rawInput: partial.rawInput,
    createdAt: 10,
    updatedAt: 20,
    ...partial,
    kind: "toolCall",
  };
}

/** Builds one turn with a stable user message for index tests. */
function turn(
  id: string,
  items: ChatToolCall[],
  status: ChatTurn["status"] = "completed",
): ChatTurn {
  return {
    id,
    userMessage: {
      kind: "message",
      id: `${id}-user`,
      role: "user",
      content: "prompt",
      createdAt: 1,
    },
    items,
    status,
    stopReason: null,
    error: null,
    createdAt: 1,
  };
}

describe("collectSessionArtifactIndex", () => {
  it("classifies protocol diffs as edited and unread locations as referenced", () => {
    const index = collectSessionArtifactIndex([
      turn("t1", [
        tool({
          id: "edit-1",
          toolKind: "edit",
          content: [
            {
              type: "diff",
              path: "src/main.rs",
              oldText: "a",
              newText: "b",
            },
          ],
          locations: [{ path: "src/main.rs" }],
        }),
        tool({
          id: "read-1",
          toolKind: "read",
          locations: [{ path: "src/lib.rs" }],
        }),
      ]),
    ]);

    expect(index).toEqual({
      edited: ["src/main.rs"],
      referenced: ["src/lib.rs"],
    });
  });

  it("includes in-progress diffs and keeps edited disjoint from referenced", () => {
    const index = collectSessionArtifactIndex([
      turn(
        "t1",
        [
          tool({
            id: "edit-live",
            toolKind: "edit",
            status: "in_progress",
            content: [
              { type: "diff", path: "src/app.ts", oldText: "", newText: "x" },
            ],
            locations: [{ path: "src/app.ts" }],
          }),
        ],
        "streaming",
      ),
    ]);

    expect(index).toEqual({
      edited: ["src/app.ts"],
      referenced: [],
    });
  });

  it("uses edit rawInput path fallbacks when ACP omitted a diff", () => {
    const index = collectSessionArtifactIndex([
      turn("t1", [
        tool({
          id: "write-1",
          toolKind: "edit",
          content: [],
          locations: [],
          rawInput: { filePath: "src/new.ts", content: "export {}\n" },
        }),
      ]),
    ]);

    expect(index.edited).toEqual(["src/new.ts"]);
  });

  it("keeps an earlier read-only turn on Files after a later edit of the same path", () => {
    const indices = collectCumulativeArtifactIndices([
      turn("t1", [
        tool({
          id: "read-1",
          toolKind: "read",
          locations: [{ path: "src/main.rs" }],
        }),
      ]),
      turn("t2", [
        tool({
          id: "edit-1",
          toolKind: "edit",
          content: [
            { type: "diff", path: "src/main.rs", oldText: "a", newText: "b" },
          ],
          locations: [{ path: "src/main.rs" }],
        }),
      ]),
    ]);

    expect(indices).toEqual([
      { edited: [], referenced: ["src/main.rs"] },
      { edited: ["src/main.rs"], referenced: [] },
    ]);
  });

  it("lets a later edit win over an earlier read in the session-wide snapshot", () => {
    const index = collectSessionArtifactIndex([
      turn("t1", [
        tool({
          id: "read-1",
          toolKind: "read",
          locations: [{ path: "src/main.rs" }],
        }),
      ]),
      turn("t2", [
        tool({
          id: "edit-1",
          toolKind: "edit",
          content: [
            { type: "diff", path: "src/main.rs", oldText: "a", newText: "b" },
          ],
          locations: [{ path: "src/main.rs" }],
        }),
      ]),
    ]);

    expect(index).toEqual({
      edited: ["src/main.rs"],
      referenced: [],
    });
  });

  it("indexes slash-terminated directory locations as directories", () => {
    const index = collectSessionArtifactIndex([
      turn("t1", [
        tool({
          id: "list-1",
          toolKind: "read",
          locations: [{ path: "src/" }],
        }),
      ]),
    ]);

    expect(index).toEqual({
      edited: [],
      referenced: [],
      directories: ["src"],
    });
  });

  it("keeps ambiguous directory-listing entries unresolved", () => {
    const index = collectSessionArtifactIndex([
      turn("t1", [
        tool({
          id: "list-1",
          title: "Get-ChildItem -Name",
          toolKind: "read",
          content: [
            {
              type: "content",
              content: {
                type: "text",
                text: "C:\\Users\\zhans\\projects\\hapi\\cli\ndocs/\nREADME.md",
              },
            },
          ],
        }),
      ]),
    ]);

    expect(index).toEqual({
      edited: [],
      referenced: [],
      directories: ["docs"],
      unknown: ["C:/Users/zhans/projects/hapi/cli", "README.md"],
    });
  });

  it("reuses completed-turn results from the per-turn cache", () => {
    const cache = new Map();
    const completed = turn("done", [
      tool({
        id: "edit-1",
        toolKind: "edit",
        content: [
          { type: "diff", path: "src/a.ts", oldText: "", newText: "a" },
        ],
      }),
    ]);
    const streaming = turn(
      "live",
      [
        tool({
          id: "read-1",
          toolKind: "read",
          status: "in_progress",
          locations: [{ path: "src/b.ts" }],
        }),
      ],
      "streaming",
    );

    collectSessionArtifactIndex([completed, streaming], cache);
    const cached = cache.get("done");
    collectSessionArtifactIndex(
      [
        completed,
        turn(
          "live",
          [
            tool({
              id: "read-1",
              toolKind: "read",
              status: "completed",
              locations: [{ path: "src/b.ts" }, { path: "src/c.ts" }],
            }),
          ],
          "streaming",
        ),
      ],
      cache,
    );

    expect(cache.get("done")).toBe(cached);
    expect(cache.get("live")?.referenced).toEqual(["src/b.ts", "src/c.ts"]);
  });

  it("reuses historical cumulative snapshots when only the live turn changes", () => {
    const cache = new Map();
    const completed = turn("done", [
      tool({ id: "read-a", locations: [{ path: "src/a.ts" }] }),
    ]);
    const first = collectCumulativeArtifactIndices(
      [completed, turn("live", [], "streaming")],
      cache,
    );
    const second = collectCumulativeArtifactIndices(
      [
        completed,
        turn(
          "live",
          [tool({ id: "read-b", locations: [{ path: "src/b.ts" }] })],
          "streaming",
        ),
      ],
      cache,
    );

    expect(second[0]).toBe(first[0]);
    expect(second[1]?.referenced).toEqual(["src/a.ts", "src/b.ts"]);
  });

  it("does not reparse tools when only assistant text changes in the live turn", () => {
    const cache = new Map();
    const live = turn(
      "live",
      [tool({ id: "read", locations: [{ path: "src/a.ts" }] })],
      "streaming",
    );
    const first = collectCumulativeArtifactIndices([live], cache);
    const cached = cache.get("live");
    const withAssistantText: ChatTurn = {
      ...live,
      items: [
        ...live.items,
        {
          kind: "message",
          id: "assistant",
          role: "assistant",
          content: "streaming answer",
          createdAt: 30,
        },
      ],
    };
    const second = collectCumulativeArtifactIndices([withAssistantText], cache);

    expect(cache.get("live")).toBe(cached);
    expect(second[0]).toBe(first[0]);
  });

  it("lets explicit file evidence replace an earlier unresolved guess", () => {
    const index = collectSessionArtifactIndex([
      turn("guess", [
        tool({
          id: "list",
          title: "Get-ChildItem -Name",
          toolKind: "execute",
          rawInput: { command: "Get-ChildItem -Name" },
          content: [
            {
              type: "content",
              content: { type: "text", text: "install\ncli" },
            },
          ],
        }),
      ]),
      turn("confirm", [
        tool({
          id: "mode",
          title: "Get-ChildItem | Select Mode, Name",
          toolKind: "execute",
          content: [
            {
              type: "content",
              content: { type: "text", text: "-a---- install\nd----- cli" },
            },
          ],
        }),
      ]),
    ]);

    expect(index).toEqual({
      edited: [],
      referenced: ["install"],
      directories: ["cli"],
    });
  });

  it("uses structured provider kinds over ambiguous visible listing text", () => {
    const index = collectSessionArtifactIndex([
      turn("typed", [
        tool({
          id: "typed-list",
          title: "Get-ChildItem -Name",
          toolKind: "execute",
          rawInput: { command: "Get-ChildItem -Name" },
          content: [
            {
              type: "content",
              content: {
                type: "text",
                text: "cache.v1\nscripts/install",
              },
            },
          ],
          rawOutput: {
            entries: [
              { path: "cache.v1", type: "directory" },
              { path: "scripts/install", kind: "file" },
            ],
          },
        }),
      ]),
    ]);

    expect(index).toEqual({
      edited: [],
      referenced: ["scripts/install"],
      directories: ["cache.v1"],
    });
  });

  it("lets later explicit directory evidence replace an earlier file guess", () => {
    const index = collectSessionArtifactIndex([
      turn("guess", [
        tool({
          id: "guess",
          toolKind: "search",
          content: [
            {
              type: "content",
              content: { type: "text", text: "cache.v1" },
            },
          ],
        }),
      ]),
      turn("confirm", [
        tool({
          id: "confirm",
          rawOutput: { path: "cache.v1", type: "directory" },
        }),
      ]),
    ]);
    expect(index).toEqual({
      edited: [],
      referenced: [],
      directories: ["cache.v1"],
    });
  });

  it("keeps extensionless provider locations unresolved", () => {
    const index = collectSessionArtifactIndex([
      turn("location", [
        tool({
          id: "location",
          toolKind: "read",
          locations: [{ path: "scripts/install" }],
        }),
      ]),
    ]);
    expect(index).toEqual({
      edited: [],
      referenced: [],
      unknown: ["scripts/install"],
    });
  });

  it("ignores failed and cancelled tool calls", () => {
    const index = collectSessionArtifactIndex([
      turn("t1", [
        tool({
          id: "edit-failed",
          toolKind: "edit",
          status: "failed",
          content: [
            { type: "diff", path: "src/fail.ts", oldText: "", newText: "f" },
          ],
          locations: [{ path: "src/fail.ts" }],
        }),
        tool({
          id: "read-cancelled",
          toolKind: "read",
          status: "cancelled",
          locations: [{ path: "src/cancel.ts" }],
        }),
      ]),
    ]);

    expect(index).toEqual({ edited: [], referenced: [] });
  });

  it("indexes markdown paths listed in a glob/search tool text dump", () => {
    const index = collectSessionArtifactIndex([
      turn("t1", [
        tool({
          id: "glob-md",
          toolKind: "search",
          locations: [],
          content: [
            {
              type: "content",
              content: {
                type: "text",
                text: "D:\\project\\desktop\\README.md\nD:\\project\\desktop\\docs\\guide.md\nD:\\project\\desktop\\crates\\engine\\README.md",
              },
            },
          ],
        }),
      ]),
    ]);

    expect(index.referenced).toEqual([
      "D:/project/desktop/README.md",
      "D:/project/desktop/docs/guide.md",
      "D:/project/desktop/crates/engine/README.md",
    ]);
  });

  it("indexes glob text paths even when locations only record the search directory", () => {
    const index = collectSessionArtifactIndex([
      turn("t1", [
        tool({
          id: "glob-md",
          toolKind: "search",
          locations: [{ path: "D:\\project\\desktop" }],
          content: [
            {
              type: "content",
              content: {
                type: "text",
                text: "README.md\ndocs/guide.md\ncrates/engine/README.md",
              },
            },
          ],
        }),
      ]),
    ]);

    expect(index.referenced).toEqual([
      "README.md",
      "docs/guide.md",
      "crates/engine/README.md",
    ]);
  });

  it("does not index glob patterns or shell commands from tool text", () => {
    const index = collectSessionArtifactIndex([
      turn("t1", [
        tool({
          id: "glob-md",
          toolKind: "search",
          locations: [],
          rawInput: { path: "**/*.md", glob: "**/*.md" },
          content: [
            {
              type: "content",
              content: {
                type: "text",
                text: "**/*.md\ncargo test\nOption<T>\nREADME.md",
              },
            },
          ],
        }),
      ]),
    ]);

    expect(index.referenced).toEqual(["README.md"]);
  });

  it("indexes filename arrays in search rawOutput", () => {
    const index = collectSessionArtifactIndex([
      turn("t1", [
        tool({
          id: "glob-md",
          toolKind: "search",
          locations: [],
          rawOutput: { filenames: ["README.md", "docs/guide.md"] },
        }),
      ]),
    ]);

    expect(index.referenced).toEqual(["README.md", "docs/guide.md"]);
  });

  it("strips file: URIs from glob dumps before indexing", () => {
    const index = collectSessionArtifactIndex([
      turn("t1", [
        tool({
          id: "glob-md",
          toolKind: "search",
          locations: [],
          content: [
            {
              type: "content",
              content: {
                type: "text",
                text: "file:///D:/project/desktop/docs/guide.md",
              },
            },
          ],
        }),
      ]),
    ]);

    expect(index.referenced).toEqual(["D:/project/desktop/docs/guide.md"]);
  });

  it("extracts referenced paths from read tool rawInput when locations are empty", () => {
    const index = collectSessionArtifactIndex([
      turn("t1", [
        tool({
          id: "read-1",
          toolKind: "read",
          locations: [],
          rawInput: {
            filePath:
              "packages/app-shell/src/features/chat/chat-link/chat-file-link.test.tsx",
          },
        }),
      ]),
    ]);

    expect(index.referenced).toEqual([
      "packages/app-shell/src/features/chat/chat-link/chat-file-link.test.tsx",
    ]);
  });

  it("never reads diff text or calls collectTurnDiffFiles while indexing", () => {
    const collect = vi.spyOn(turnDiffFiles, "collectTurnDiffFiles");
    const throwingDiff = {
      type: "diff" as const,
      path: "src/main.rs",
      get oldText(): string {
        throw new Error("must not read oldText");
      },
      get newText(): string {
        throw new Error("must not read newText");
      },
    };

    expect(() =>
      collectSessionArtifactIndex([
        turn("t1", [
          tool({
            id: "edit-1",
            toolKind: "edit",
            content: [throwingDiff],
            locations: [{ path: "src/main.rs" }],
          }),
        ]),
      ]),
    ).not.toThrow();
    expect(collect).not.toHaveBeenCalled();
    collect.mockRestore();
  });

  it("indexes one-path-per-line dumps from execute tools", () => {
    const index = collectSessionArtifactIndex([
      turn("t1", [
        tool({
          id: "ls-md",
          title: "Search markdown output",
          toolKind: "execute",
          locations: [],
          content: [
            {
              type: "content",
              content: {
                type: "text",
                text: "README.md\ndocs/guide.md",
              },
            },
          ],
        }),
      ]),
    ]);

    expect(index.referenced).toEqual(["README.md", "docs/guide.md"]);
  });
});
