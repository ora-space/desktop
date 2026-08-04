import { describe, expect, it } from "vitest";
import { parseDiff } from "react-diff-view";
import {
  buildDiffFileTree,
  diffFilePath,
  filterDiffFiles,
} from "./task-diff-file-tree";

const PATCH = [
  "diff --git a/packages/app/src/main.ts b/packages/app/src/main.ts",
  "index 3bd1f0e..17c13d8 100644",
  "--- a/packages/app/src/main.ts",
  "+++ b/packages/app/src/main.ts",
  "@@ -1 +1 @@",
  "-const value = 1;",
  "+const value = 2;",
  "diff --git a/packages/sdk/README.md b/packages/sdk/README.md",
  "deleted file mode 100644",
  "index d6fa8e1..0000000",
  "--- a/packages/sdk/README.md",
  "+++ /dev/null",
  "@@ -1 +0,0 @@",
  "-# SDK",
  "",
].join("\n");

describe("task diff file tree", () => {
  it("groups changed files by directory and keeps deleted paths selectable", () => {
    const files = parseDiff(PATCH);

    expect(buildDiffFileTree(files)).toMatchObject([
      {
        kind: "directory",
        name: "packages",
        path: "packages",
        children: [
          {
            kind: "directory",
            name: "app",
            path: "packages/app",
            children: [
              {
                kind: "directory",
                name: "src",
                path: "packages/app/src",
                children: [{ kind: "file", name: "main.ts", path: "packages/app/src/main.ts" }],
              },
            ],
          },
          {
            kind: "directory",
            name: "sdk",
            path: "packages/sdk",
            children: [{ kind: "file", name: "README.md", path: "packages/sdk/README.md" }],
          },
        ],
      },
    ]);
    expect(diffFilePath(files[1]!)).toBe("packages/sdk/README.md");
  });

  it("filters files by complete path without matching case", () => {
    const files = parseDiff(PATCH);

    expect(filterDiffFiles(files, "SDK/readme")).toEqual([files[1]]);
    expect(filterDiffFiles(files, "  ")).toEqual(files);
  });
});
