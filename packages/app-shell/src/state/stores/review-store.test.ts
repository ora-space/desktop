import { beforeEach, describe, expect, it } from "vitest";
import {
  buildReviewFilePersist,
  clampReviewWidth,
  REVIEW_STORAGE_KEY,
  reviewContextKey,
  sanitizeReviewContextPersist,
  useReviewStore,
} from "./review-store";
import { flushDebouncedPersistStorage } from "./debounced-json-storage";
import { DEFAULT_REVIEW_WIDTH } from "../../features/workspace/workspace-review-layout-utils";

beforeEach(() => {
  flushDebouncedPersistStorage();
  window.localStorage.clear();
  useReviewStore.setState({ byContext: {} });
  flushDebouncedPersistStorage();
  window.localStorage.clear();
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
        fileRequest: { path: "/repo/src/main.ts", line: 12 },
      }),
    ).toEqual({ path: "src/main.ts", line: 12 });
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
    });
    expect(
      sanitizeReviewContextPersist({
        open: true,
        panel: "changes",
        width: 99999,
        file: { path: "", line: "bad" },
      }),
    ).toEqual({
      open: true,
      panel: "changes",
      width: clampReviewWidth(99999),
      file: undefined,
    });
  });

  it("round-trips one context through localStorage", async () => {
    window.localStorage.setItem(
      REVIEW_STORAGE_KEY,
      JSON.stringify({
        state: {
          byContext: {
            "task:t1": {
              open: true,
              panel: "changes",
              width: 720,
              file: { path: "src/main.ts", line: 12 },
            },
          },
        },
        version: 0,
      }),
    );

    await useReviewStore.persist.rehydrate();

    expect(useReviewStore.getState().byContext["task:t1"]).toEqual({
      open: true,
      panel: "changes",
      width: 720,
      file: { path: "src/main.ts", line: 12 },
    });
  });

  it("keeps in-memory edits when async rehydrate finishes later", async () => {
    window.localStorage.setItem(
      REVIEW_STORAGE_KEY,
      JSON.stringify({
        state: {
          byContext: {
            "task:t1": {
              open: false,
              panel: "files",
              width: DEFAULT_REVIEW_WIDTH,
            },
          },
        },
        version: 0,
      }),
    );
    useReviewStore.getState().upsertContext("task:t1", {
      open: true,
      panel: "changes",
      width: 640,
    });

    await useReviewStore.persist.rehydrate();

    expect(useReviewStore.getState().byContext["task:t1"]).toEqual({
      open: true,
      panel: "changes",
      width: 640,
    });
  });
});
