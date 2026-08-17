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
});
