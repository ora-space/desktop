import { describe, expect, it } from "vitest";
import { classifyChatCandidate } from "./classify";
import type { SessionArtifactIndex } from "./artifact-index";

const index: SessionArtifactIndex = {
  edited: ["src/main.rs"],
  referenced: ["src/lib.rs", "README.md"],
};

describe("classifyChatCandidate", () => {
  it("routes edited inline paths to Diff and referenced paths to Files", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "src/main.rs",
        index,
        hasNavigation: true,
      }),
    ).toMatchObject({ kind: "diff", path: "src/main.rs" });
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "src/lib.rs",
        index,
        hasNavigation: true,
      }),
    ).toMatchObject({ kind: "files", path: "src/lib.rs" });
  });

  it("routes absolute and slash-terminated directory output to the Files tree", () => {
    const directoryIndex: SessionArtifactIndex = {
      edited: [],
      referenced: [],
      directories: ["C:/repo/cli", "docs"],
    };
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "C:\\repo\\cli",
        index: directoryIndex,
        hasNavigation: true,
        cwd: "C:/repo",
      }),
    ).toMatchObject({ kind: "directory", path: "cli" });
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "docs/",
        index: directoryIndex,
        hasNavigation: true,
      }),
    ).toMatchObject({ kind: "directory", path: "docs" });
  });

  it("prefers explicit file evidence over directory heuristics and conflicts", () => {
    expect(
      classifyChatCandidate({
        source: "href",
        raw: "scripts/install",
        index: {
          edited: [],
          referenced: ["scripts/install"],
          directories: ["scripts/install"],
        },
        hasNavigation: true,
      }),
    ).toMatchObject({ kind: "files", path: "scripts/install" });
  });

  it("links a directory the index holds both qualified and bare", () => {
    // One listing reaches the index twice: qualified with its listing root from
    // the visible output, and bare from `rawOutput`. Both name the same entry.
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "docs",
        index: {
          edited: [],
          referenced: ["C:/repo/main.py", "main.py"],
          directories: ["C:/repo/docs", "docs"],
        },
        hasNavigation: true,
        cwd: "C:/repo",
      }),
    ).toMatchObject({ kind: "directory", path: "docs" });
  });

  it("keeps only explicit trailing-slash href misses as directories", () => {
    expect(
      classifyChatCandidate({
        source: "href",
        raw: "scripts/install",
        index: { edited: [], referenced: [] },
        hasNavigation: true,
      }),
    ).toMatchObject({ kind: "files", path: "scripts/install" });
    expect(
      classifyChatCandidate({
        source: "href",
        raw: "scripts/generated/",
        index: { edited: [], referenced: [] },
        hasNavigation: true,
      }),
    ).toMatchObject({ kind: "directory", path: "scripts/generated/" });
  });

  it("opens an indexed checkout root as the Files root directory", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "C:/repo",
        index: {
          edited: [],
          referenced: [],
          directories: ["C:/repo"],
        },
        hasNavigation: true,
        cwd: "C:/repo",
      }),
    ).toMatchObject({ kind: "directory", path: "" });
  });

  it("keeps commands and type names as plain code", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "cargo test",
        index,
        hasNavigation: true,
      }),
    ).toEqual({ kind: "none" });
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "Option<T>",
        index,
        hasNavigation: true,
      }),
    ).toEqual({ kind: "none" });
  });

  it("links a unique bare filename to the index path, not the typed token", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "main.rs",
        index,
        hasNavigation: true,
      }),
    ).toMatchObject({ kind: "diff", path: "src/main.rs" });
  });

  it("does not link an ambiguous bare filename", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "main.rs",
        index: {
          edited: ["src/main.rs", "crates/app/src/main.rs"],
          referenced: [],
        },
        hasNavigation: true,
      }),
    ).toEqual({ kind: "none" });
  });

  it("does not link a bare filename that appears once in edited and once in referenced", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "main.rs",
        index: {
          edited: ["src/main.rs"],
          referenced: ["tests/main.rs"],
        },
        hasNavigation: true,
      }),
    ).toEqual({ kind: "none" });
  });

  it("links a bare README.md to the workspace-root file when nested copies also exist", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "README.md",
        index: {
          edited: [],
          referenced: ["README.md", "packages/app-shell/README.md"],
        },
        hasNavigation: true,
      }),
    ).toMatchObject({ kind: "files", path: "README.md" });
  });

  it("links a bare README.md to the shortest absolute root file without a cwd", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "README.md",
        index: {
          edited: [],
          referenced: [
            "D:/project/desktop/README.md",
            "D:/project/desktop/crates/engine/README.md",
          ],
        },
        hasNavigation: true,
        cwd: "D:/project/desktop",
      }),
    ).toMatchObject({ kind: "files", path: "README.md" });
  });

  it("does not guess among nested README.md files when no workspace-root copy exists", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "README.md",
        index: {
          edited: [],
          referenced: ["crates/a/README.md", "crates/b/README.md"],
        },
        hasNavigation: true,
      }),
    ).toEqual({ kind: "none" });
  });

  it("still opens Files for an explicit href when the bare filename is ambiguous", () => {
    expect(
      classifyChatCandidate({
        source: "href",
        raw: "README.md",
        index: {
          edited: [],
          referenced: ["README.md", "packages/app-shell/README.md"],
        },
        hasNavigation: true,
      }),
    ).toMatchObject({ kind: "files", path: "README.md" });
  });

  it("sends explicit file hrefs that miss the index to Files", () => {
    expect(
      classifyChatCandidate({
        source: "href",
        raw: "docs/guide.md",
        index,
        hasNavigation: true,
      }),
    ).toMatchObject({ kind: "files", path: "docs/guide.md" });
  });

  it("keeps http(s) as web links and ignores dangerous schemes", () => {
    expect(
      classifyChatCandidate({
        source: "href",
        raw: "https://example.com",
        index,
        hasNavigation: true,
      }),
    ).toEqual({ kind: "web", href: "https://example.com" });
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "https://example.com",
        index,
        hasNavigation: true,
      }),
    ).toEqual({ kind: "web", href: "https://example.com" });
    expect(
      classifyChatCandidate({
        source: "href",
        raw: "javascript:alert(1)",
        index,
        hasNavigation: true,
      }),
    ).toEqual({ kind: "none" });
  });

  it("does not link when the review layout has no navigation", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "src/main.rs",
        index,
        hasNavigation: false,
      }),
    ).toEqual({ kind: "none" });
  });

  it("strips the task cwd from an absolute ACP path before opening", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "src/main.rs",
        index: { edited: ["C:/Repo/src/main.rs"], referenced: [] },
        hasNavigation: true,
        cwd: "C:/Repo",
      }),
    ).toMatchObject({ kind: "diff", path: "src/main.rs", line: undefined });
  });

  it("passes parsed line numbers through to Diff", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "src/main.rs:12",
        index,
        hasNavigation: true,
      }),
    ).toMatchObject({ kind: "diff", path: "src/main.rs", line: 12 });
  });

  it("links a bare filename to the full relative path in the index", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "chat-file-link.test.tsx",
        index: {
          edited: [],
          referenced: [
            "packages/app-shell/src/features/chat/chat-link/chat-file-link.test.tsx",
          ],
        },
        hasNavigation: true,
      }),
    ).toMatchObject({
      kind: "files",
      path: "packages/app-shell/src/features/chat/chat-link/chat-file-link.test.tsx",
    });
  });

  it("strips cwd when linking a bare filename to an absolute index path", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "chat-file-link.test.tsx",
        index: {
          edited: [],
          referenced: [
            "E:/claude_code_project/desktop/packages/app-shell/src/features/chat/chat-link/chat-file-link.test.tsx",
          ],
        },
        hasNavigation: true,
        cwd: "E:/claude_code_project/desktop",
      }),
    ).toMatchObject({
      kind: "files",
      path: "packages/app-shell/src/features/chat/chat-link/chat-file-link.test.tsx",
    });
  });

  it("does not open Files for a suffix match of a file read outside the task cwd", () => {
    expect(
      classifyChatCandidate({
        source: "inline-code",
        raw: "crates/acp/src/lib.rs",
        index: {
          edited: [],
          referenced: ["D:/project/desktop/crates/acp/src/lib.rs"],
        },
        hasNavigation: true,
        cwd: "D:/project/desktop/.data/worktrees/f06fdb43-1297-4ba3-9143-a7a95ee85b0b",
      }),
    ).toEqual({ kind: "none" });
  });

  it("does not open Files for a Markdown href that only suffix-matches an outside-cwd path", () => {
    expect(
      classifyChatCandidate({
        source: "href",
        raw: "crates/acp/src/lib.rs",
        index: {
          edited: [],
          referenced: ["D:/project/desktop/crates/acp/src/lib.rs"],
        },
        hasNavigation: true,
        cwd: "D:/project/desktop/.data/worktrees/f06fdb43-1297-4ba3-9143-a7a95ee85b0b",
      }),
    ).toEqual({ kind: "none" });
  });

  it("does not link an absolute Markdown href outside the task cwd", () => {
    expect(
      classifyChatCandidate({
        source: "href",
        raw: "C:/other/config.toml",
        index,
        hasNavigation: true,
        cwd: "C:/repo",
      }),
    ).toEqual({ kind: "none" });
  });

  it.each(["../outside.txt", "%2e%2e/outside.txt", "src/../../outside.txt"])(
    "rejects parent traversal before Files or OS navigation: %s",
    (raw) => {
      expect(
        classifyChatCandidate({
          source: "href",
          raw,
          index,
          hasNavigation: true,
          cwd: "C:/repo",
        }),
      ).toEqual({ kind: "none" });
    },
  );
});
