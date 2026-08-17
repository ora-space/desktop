import { describe, expect, it } from "vitest";
import {
  isAbsoluteWorkspacePath,
  joinOsAbsolutePath,
  normalizeDiffPath,
  pathsMatchForWorkspace,
  stripTaskCwdPrefix,
} from "./workspace-path";

describe("normalizeDiffPath", () => {
  it("converts backslashes and strips a leading ./ or / segment marker", () => {
    expect(normalizeDiffPath("src\\main.rs")).toBe("src/main.rs");
    expect(normalizeDiffPath("./src/main.rs")).toBe("src/main.rs");
    expect(normalizeDiffPath("/src/main.rs")).toBe("src/main.rs");
  });
});

describe("pathsMatchForWorkspace", () => {
  it("matches equal paths after slash normalization", () => {
    expect(pathsMatchForWorkspace("src\\main.rs", "src/main.rs")).toBe(true);
  });

  it("matches either side as a suffix of the other", () => {
    expect(pathsMatchForWorkspace("src/main.rs", "main.rs")).toBe(true);
    expect(pathsMatchForWorkspace("main.rs", "src/main.rs")).toBe(true);
    expect(pathsMatchForWorkspace("src/main.rs", "lib.rs")).toBe(false);
  });

  it("compares case-insensitively for Windows worktrees", () => {
    expect(pathsMatchForWorkspace("Src/Main.rs", "src/main.rs")).toBe(true);
    expect(pathsMatchForWorkspace("C:/Repo/src/Main.rs", "src/main.rs")).toBe(
      true,
    );
  });
});

describe("isAbsoluteWorkspacePath", () => {
  it("detects Windows drive and POSIX rooted paths after normalization", () => {
    expect(isAbsoluteWorkspacePath("C:\\repo\\src\\main.rs")).toBe(true);
    expect(isAbsoluteWorkspacePath("/repo/src/main.rs")).toBe(true);
    expect(isAbsoluteWorkspacePath("src/main.rs")).toBe(false);
  });
});

describe("stripTaskCwdPrefix", () => {
  it("returns the workspace-relative remainder with original casing", () => {
    expect(stripTaskCwdPrefix("C:\\Repo\\src\\Main.rs", "c:/repo")).toBe(
      "src/Main.rs",
    );
  });

  it("returns null when the path is not under the cwd", () => {
    expect(stripTaskCwdPrefix("D:/other/src/main.rs", "C:/repo")).toBeNull();
  });
});

describe("joinOsAbsolutePath", () => {
  it("keeps an already-absolute tool path and joins a relative path with the cwd", () => {
    expect(joinOsAbsolutePath("C:/repo/src/main.rs", "C:/repo")).toBe(
      "C:/repo/src/main.rs",
    );
    expect(joinOsAbsolutePath("src/main.rs", "C:/repo/")).toBe(
      "C:/repo/src/main.rs",
    );
  });
});
