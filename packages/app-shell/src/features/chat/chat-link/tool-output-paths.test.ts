import { describe, expect, it } from "vitest";
import type { ChatToolCall } from "@ora/chat";
import {
  collectToolOutputPaths,
  extractArtifactPathsFromText,
  isPlainPathList,
  stripListMarker,
} from "./tool-output-paths";

describe("extractArtifactPathsFromText", () => {
  it("keeps concrete file lines and drops commands and globs", () => {
    expect(
      extractArtifactPathsFromText(
        "D:\\project\\desktop\\README.md\n**/*.md\ncargo test\n- docs/guide.md",
      ),
    ).toEqual(["D:\\project\\desktop\\README.md", "docs/guide.md"]);
    expect(
      extractArtifactPathsFromText("file:///D:/project/desktop/docs/guide.md"),
    ).toEqual(["D:/project/desktop/docs/guide.md"]);
  });
});

describe("collectToolOutputPaths", () => {
  it("reads filename arrays from search rawOutput", () => {
    const tool: ChatToolCall = {
      kind: "toolCall",
      id: "glob-1",
      title: "Glob",
      toolKind: "search",
      status: "completed",
      content: [],
      locations: [],
      rawOutput: { filenames: ["README.md", "docs/guide.md"] },
      createdAt: 1,
      updatedAt: 1,
    };
    expect(collectToolOutputPaths(tool)).toEqual([
      "README.md",
      "docs/guide.md",
    ]);
  });

  it("reads path arrays stored under singular provider keys", () => {
    const tool: ChatToolCall = {
      kind: "toolCall",
      id: "glob-2",
      title: "Glob",
      toolKind: "search",
      status: "completed",
      content: [],
      locations: [],
      rawOutput: { file: ["README.md", "docs/guide.md"] },
      createdAt: 1,
      updatedAt: 1,
    };
    expect(collectToolOutputPaths(tool)).toEqual([
      "README.md",
      "docs/guide.md",
    ]);
  });
});

describe("isPlainPathList", () => {
  it("accepts text fences that are only file paths", () => {
    expect(isPlainPathList("README.md\ndocs/guide.md", "text")).toBe(true);
    expect(isPlainPathList("fn main() {}", "rust")).toBe(false);
    expect(isPlainPathList("cargo test\nREADME.md", "text")).toBe(false);
  });
});

describe("stripListMarker", () => {
  it("strips bullets, numbers, and task boxes", () => {
    expect(stripListMarker("- README.md")).toBe("README.md");
    expect(stripListMarker("1. docs/guide.md")).toBe("docs/guide.md");
    expect(stripListMarker("[ ] crates/engine/README.md")).toBe(
      "crates/engine/README.md",
    );
  });
});
