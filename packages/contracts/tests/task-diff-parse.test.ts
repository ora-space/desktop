import assert from "node:assert/strict";
import test from "node:test";
import { parseDiff } from "react-diff-view";

const BACKEND_PATCH = [
  "diff --git a/untracked.txt b/untracked.txt",
  "new file mode 100644",
  "index 0000000..ce01362",
  "--- /dev/null",
  "+++ b/untracked.txt",
  "@@ -0,0 +1 @@",
  "+hello",
  "diff --git a/empty.txt b/empty.txt",
  "new file mode 100644",
  "index 0000000..e69de29",
  "diff --git a/binary.bin b/binary.bin",
  "new file mode 100644",
  "index 0000000..2e45efe",
  "Binary files /dev/null and b/binary.bin differ",
  "",
].join("\n");

test("react-diff-view retains text and metadata-only task diff files", () => {
  const files = parseDiff(BACKEND_PATCH);

  assert.equal(files.length, 3);
  assert.deepEqual(
    files.map(({ newPath, newRevision, hunks }) => ({
      newPath,
      newRevision,
      hunkCount: hunks.length,
    })),
    [
      { newPath: "untracked.txt", newRevision: "ce01362", hunkCount: 1 },
      { newPath: "empty.txt", newRevision: "e69de29", hunkCount: 0 },
      { newPath: "binary.bin", newRevision: "2e45efe", hunkCount: 0 },
    ],
  );

  const [change] = files[0].hunks[0].changes;
  assert.equal(change.type, "insert");
  if (change.type === "insert") {
    assert.equal(change.lineNumber, 1);
  }
});
