import { beforeEach, describe, expect, it } from "vitest";
import {
  buildReviewFilePersist,
  clampReviewWidth,
  reviewContextKey,
  sanitizeReviewContextPersist,
  useReviewStore,
} from "./review-store";
import { DEFAULT_REVIEW_WIDTH } from "./review-layout";

beforeEach(() => {
  useReviewStore.setState({ byContext: {} });
});

describe("review-store", () => {
  it("builds stable keys per checkout scope", () => {
    expect(reviewContextKey({ kind: "project", projectId: "p1" })).toBe(
      "project:p1",
    );
    expect(
      reviewContextKey({
        kind: "task",
        projectId: "p1",
        taskId: "t1",
      }),
    ).toBe("task:t1");
    expect(reviewContextKey({ kind: "none" })).toBeNull();
  });

  it("keeps line metadata when preview and request paths match by suffix", () => {
    expect(
      buildReviewFilePersist({
        open: true,
        panel: "changes",
        reviewFilePath: "src/main.ts",
        fileRequest: {
          path: "/repo/src/main.ts",
          line: 12,
          endLine: 18,
          side: "old",
        },
      }),
    ).toEqual({
      path: "src/main.ts",
      line: 12,
      endLine: 18,
      side: "old",
    });
  });

  it("omits line metadata when preview and request paths do not match", () => {
    expect(
      buildReviewFilePersist({
        open: true,
        panel: "changes",
        reviewFilePath: "src/other.ts",
        fileRequest: { path: "src/main.ts", line: 12 },
      }),
    ).toEqual({ path: "src/other.ts" });
  });

  it("returns undefined when the panel is closed", () => {
    expect(
      buildReviewFilePersist({
        open: false,
        panel: "files",
        reviewFilePath: "src/main.ts",
      }),
    ).toBeUndefined();
  });

  it("clamps corrupt widths and drops invalid file paths", () => {
    expect(sanitizeReviewContextPersist(undefined)).toEqual({
      open: false,
      panel: "files",
      width: DEFAULT_REVIEW_WIDTH,
      files: {},
    });
    expect(
      sanitizeReviewContextPersist({
        open: true,
        panel: "changes",
        width: 99999,
        files: {
          changes: { path: "", line: "bad" },
          files: { path: "src/keep.ts", line: 3 },
        },
      }),
    ).toEqual({
      open: true,
      panel: "changes",
      width: clampReviewWidth(99999),
      files: { files: { path: "src/keep.ts", line: 3 } },
    });
  });

  it("keeps each panel's preview independent", () => {
    useReviewStore.getState().upsertContext("task:t1", {
      open: true,
      panel: "files",
      files: { files: { path: "src/a.ts" } },
    });
    useReviewStore.getState().upsertContext("task:t1", {
      panel: "changes",
      files: { changes: { path: "src/b.ts", line: 9 } },
    });
    // Switching back to Files must not adopt the Changes-only path.
    useReviewStore.getState().upsertContext("task:t1", { panel: "files" });

    expect(useReviewStore.getState().byContext["task:t1"]).toEqual({
      open: true,
      panel: "files",
      width: DEFAULT_REVIEW_WIDTH,
      files: {
        files: { path: "src/a.ts" },
        changes: { path: "src/b.ts", line: 9 },
      },
    });
  });

  it("prunes scopes whose project or task no longer exists", () => {
    useReviewStore.getState().upsertContext("project:p1", { open: true });
    useReviewStore.getState().upsertContext("project:gone", { open: true });
    useReviewStore.getState().upsertContext("task:t1", { open: true });
    useReviewStore.getState().upsertContext("task:gone", { open: true });

    useReviewStore.getState().pruneContexts(["p1"], ["t1"]);

    expect(Object.keys(useReviewStore.getState().byContext).sort()).toEqual([
      "project:p1",
      "task:t1",
    ]);
  });
});
