import { describe, expect, it } from "vitest";
import { isWorkspacePathInside, uniqueCopyName } from "./unique-copy-name";

describe("uniqueCopyName", () => {
  it("keeps the original name when the folder is free", () => {
    expect(uniqueCopyName("README.md", new Set())).toBe("README.md");
  });

  it("inserts copy before the last extension like Cursor's explorer", () => {
    expect(uniqueCopyName("README.md", new Set(["README.md"]))).toBe(
      "README copy.md",
    );
    expect(
      uniqueCopyName("README.md", new Set(["README.md", "README copy.md"])),
    ).toBe("README copy 2.md");
  });

  it("treats extension-less folders as a whole stem", () => {
    expect(uniqueCopyName("src", new Set(["src"]))).toBe("src copy");
  });
});

describe("isWorkspacePathInside", () => {
  it("treats the workspace root as a parent of every path", () => {
    expect(isWorkspacePathInside("", "src/lib.rs")).toBe(true);
  });

  it("does not treat a sibling prefix as a descendant", () => {
    expect(isWorkspacePathInside("src", "src-2")).toBe(false);
    expect(isWorkspacePathInside("src", "src/lib.rs")).toBe(true);
  });
});
