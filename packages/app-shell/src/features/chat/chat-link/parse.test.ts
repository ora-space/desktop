import { describe, expect, it } from "vitest";
import {
  isLikelyFileArtifactPath,
  isPathLikeToken,
  parseChatHref,
  parsePathCandidate,
} from "./parse";

describe("isPathLikeToken", () => {
  it("rejects shell commands and type syntax", () => {
    expect(isPathLikeToken("cargo test")).toBe(false);
    expect(isPathLikeToken("Option<T>")).toBe(false);
    expect(isPathLikeToken("READY")).toBe(false);
    expect(isPathLikeToken("Summary of artifact-index.test.ts")).toBe(false);
    expect(isPathLikeToken("Step 1.2")).toBe(false);
  });

  it("accepts separators, known filenames, extensions, line suffixes, and absolute paths", () => {
    expect(isPathLikeToken("src/main.rs")).toBe(true);
    expect(isPathLikeToken("src\\main.rs")).toBe(true);
    expect(isPathLikeToken("main.rs")).toBe(true);
    expect(isPathLikeToken("Makefile")).toBe(true);
    expect(isPathLikeToken("src/main.rs:12")).toBe(true);
    expect(isPathLikeToken("C:\\repo\\src\\main.rs")).toBe(true);
    expect(isPathLikeToken("/repo/src/main.rs")).toBe(true);
    expect(isPathLikeToken("**/*.md")).toBe(false);
  });
});

describe("parsePathCandidate", () => {
  it("keeps internal spaces and decodes %20", () => {
    expect(parsePathCandidate("`src/foo bar.rs`")).toEqual({
      path: "src/foo bar.rs",
      line: undefined,
      column: undefined,
    });
    expect(parsePathCandidate("src/foo%20bar.rs")).toEqual({
      path: "src/foo bar.rs",
      line: undefined,
      column: undefined,
    });
  });

  it("strips wrapping delimiters and English or CJK prose punctuation", () => {
    expect(parsePathCandidate('"src/main.rs",')).toEqual({
      path: "src/main.rs",
      line: undefined,
      column: undefined,
    });
    expect(parsePathCandidate("(src/main.rs)。")).toEqual({
      path: "src/main.rs",
      line: undefined,
      column: undefined,
    });
  });

  it("parses colon, fragment, query, and natural-language locations", () => {
    expect(parsePathCandidate("src/main.rs:12")).toEqual({
      path: "src/main.rs",
      line: 12,
      column: undefined,
    });
    expect(parsePathCandidate("src/main.rs:12:3")).toEqual({
      path: "src/main.rs",
      line: 12,
      column: 3,
    });
    expect(parsePathCandidate("src/main.rs (line 12)")).toEqual({
      path: "src/main.rs",
      line: 12,
      column: undefined,
    });
    expect(parsePathCandidate("src/main.rs (line 12, column 3)。")).toEqual({
      path: "src/main.rs",
      line: 12,
      column: 3,
    });
    expect(parsePathCandidate("src/main.rs#L12C3")).toEqual({
      path: "src/main.rs",
      line: 12,
      column: 3,
    });
    expect(parsePathCandidate("src/main.rs?line=12&column=3")).toEqual({
      path: "src/main.rs",
      line: 12,
      column: 3,
    });
  });

  it("does not accept zero as a source location", () => {
    expect(parsePathCandidate("src/main.rs:0")).toEqual({
      path: "src/main.rs:0",
      line: undefined,
      column: undefined,
    });
  });
});

describe("parseChatHref", () => {
  it("classifies http(s) as web and file/relative paths as files", () => {
    expect(parseChatHref("https://example.com/docs")).toEqual({
      kind: "web",
      href: "https://example.com/docs",
    });
    expect(parseChatHref("file:///src/main.rs")).toEqual({
      kind: "file",
      path: "/src/main.rs",
      line: undefined,
      column: undefined,
    });
    expect(parseChatHref("./src/main.rs:12")).toEqual({
      kind: "file",
      path: "src/main.rs",
      line: 12,
      column: undefined,
    });
  });

  it("does not decode percent-escapes in http(s) hrefs", () => {
    expect(
      parseChatHref("https://example.com/v2/search?q=a%26b#frag%2Fment"),
    ).toEqual({
      kind: "web",
      href: "https://example.com/v2/search?q=a%26b#frag%2Fment",
    });
  });

  it("treats dangerous and other non-file schemes as inert", () => {
    expect(parseChatHref("javascript:alert(1)")).toEqual({ kind: "inert" });
    expect(parseChatHref("data:text/html,hi")).toEqual({ kind: "inert" });
    expect(parseChatHref("mailto:a@b.com")).toEqual({ kind: "inert" });
    expect(parseChatHref("ssh://git@github.com/user/repo.git")).toEqual({
      kind: "inert",
    });
    expect(parseChatHref("ftp://example.com/a.rs")).toEqual({ kind: "inert" });
  });

  it("decodes percent-escapes in file paths only once", () => {
    expect(parsePathCandidate("dir/file%2520name.rs")).toEqual({
      path: "dir/file%20name.rs",
      line: undefined,
      column: undefined,
    });
    expect(parseChatHref("dir/file%2520name.rs")).toEqual({
      kind: "file",
      path: "dir/file%20name.rs",
      line: undefined,
      column: undefined,
    });
  });
});

describe("isLikelyFileArtifactPath", () => {
  it("accepts files and rejects search roots and glob patterns", () => {
    expect(isLikelyFileArtifactPath("README.md")).toBe(true);
    expect(isLikelyFileArtifactPath("docs/guide.md")).toBe(true);
    expect(isLikelyFileArtifactPath("D:/project/desktop/README.md")).toBe(true);
    expect(isLikelyFileArtifactPath("D:/project/desktop")).toBe(false);
    expect(isLikelyFileArtifactPath("src/")).toBe(false);
    expect(isLikelyFileArtifactPath("**/*.md")).toBe(false);
  });
});
