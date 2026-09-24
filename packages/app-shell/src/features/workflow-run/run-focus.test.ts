import { describe, expect, it } from "vitest";
import { executableRun } from "./executable-run";
import { createMockWorkflow as createMockWorkflowFixture } from "@ora/workflow-mock";
import {
  resolveCompletionAdvanceNodeId,
  resolveFocusNodeId,
  resolveOverviewFocusedId,
  resolveStageFocusNodeId,
  resolveTheaterFocus,
  shouldAdvanceAutomaticConversation,
  shouldReleaseFocusToFollow,
  shouldReleaseLivePinToFollow,
  shouldStealFocusForArtifactReveal,
} from "./run-focus";
import {
  normalizeWorkflowDefinition,
  type GraphWorkflowRun,
} from "@ora/workflow-runtime";

function baseRun(overrides: Partial<GraphWorkflowRun> = {}): GraphWorkflowRun {
  const snapshot = normalizeWorkflowDefinition(
    createMockWorkflowFixture("zh-CN"),
  );
  return {
    id: "gwr-1",
    projectId: "p1",
    definitionId: snapshot.id,
    definitionSnapshot: snapshot,
    name: snapshot.name,
    status: "running",
    nodeStates: Object.fromEntries(
      snapshot.nodes.map((node) => [node.id, { status: "idle" as const }]),
    ),
    openHitls: [],
    createdAt: "2026-08-01T12:00:00+08:00",
    updatedAt: "2026-08-01T12:00:00+08:00",
    ...overrides,
  };
}

describe("resolveCompletionAdvanceNodeId", () => {
  it("excludes backend-designated spare nodes from Theater without changing Overview's snapshot", () => {
    const run = baseRun();
    const before = structuredClone(run);
    const unusedId = run.definitionSnapshot.nodes[1]!.id;
    const projected = executableRun(run, [unusedId]);
    expect(run).toEqual(before);
    expect(projected.definitionSnapshot).toEqual({
      ...run.definitionSnapshot,
      nodes: run.definitionSnapshot.nodes.filter(
        (node) => node.id !== unusedId,
      ),
      edges: run.definitionSnapshot.edges.filter(
        (edge) => edge.source !== unusedId && edge.target !== unusedId,
      ),
    });
    expect(projected.nodeStates).not.toHaveProperty(unusedId);
    expect(resolveTheaterFocus(projected, unusedId).activeIds).not.toContain(
      unusedId,
    );
  });
  it("waits for the completed node's refreshed terminal state", () => {
    const run = baseRun({
      nodeStates: {
        start: { status: "succeeded", finishedAt: "a" },
        understand: { status: "succeeded", finishedAt: "b" },
        quality: { status: "awaiting_input", startedAt: "c" },
        tests: { status: "running", startedAt: "d" },
        review: { status: "idle" },
        output: { status: "idle" },
      },
    });

    expect(resolveCompletionAdvanceNodeId(run, "quality")).toBeNull();
  });

  it("chooses the first parallel active node in stable path order", () => {
    const run = baseRun({
      nodeStates: {
        start: { status: "succeeded", finishedAt: "a" },
        understand: { status: "succeeded", finishedAt: "b" },
        quality: { status: "succeeded", finishedAt: "c" },
        tests: { status: "running", startedAt: "d" },
        review: { status: "running", startedAt: "d" },
        output: { status: "idle" },
      },
    });

    expect(resolveCompletionAdvanceNodeId(run, "quality")).toBe("tests");
  });

  it("returns no target when the workflow has no active successor", () => {
    const run = baseRun({
      status: "succeeded",
      nodeStates: {
        start: { status: "succeeded", finishedAt: "a" },
        understand: { status: "succeeded", finishedAt: "b" },
        quality: { status: "succeeded", finishedAt: "c" },
        tests: { status: "succeeded", finishedAt: "d" },
        review: { status: "succeeded", finishedAt: "d" },
        output: { status: "succeeded", finishedAt: "e" },
      },
    });

    expect(resolveCompletionAdvanceNodeId(run, "review")).toBeNull();
  });
});

describe("shouldReleaseFocusToFollow", () => {
  it("releases when the same live focus just became terminal", () => {
    expect(
      shouldReleaseFocusToFollow(
        { nodeId: "understand", status: "running" },
        "understand",
        "succeeded",
      ),
    ).toBe(true);
    expect(
      shouldReleaseFocusToFollow(
        { nodeId: "understand", status: "awaiting_input" },
        "understand",
        "succeeded",
      ),
    ).toBe(true);
    expect(
      shouldReleaseFocusToFollow(
        { nodeId: "understand", status: "running" },
        "understand",
        "failed",
      ),
    ).toBe(true);
    expect(
      shouldReleaseFocusToFollow(
        { nodeId: "understand", status: "running" },
        "understand",
        "cancelled",
      ),
    ).toBe(true);
  });

  it("does not release when switching to another already-finished node", () => {
    expect(
      shouldReleaseFocusToFollow(
        { nodeId: "understand", status: "running" },
        "quality",
        "succeeded",
      ),
    ).toBe(false);
  });

  it("does not release a history pin while something else is live", () => {
    expect(
      shouldReleaseFocusToFollow(
        { nodeId: "start", status: "succeeded" },
        "start",
        "succeeded",
      ),
    ).toBe(false);
    expect(
      shouldReleaseFocusToFollow(
        { nodeId: "output", status: "idle" },
        "output",
        "idle",
      ),
    ).toBe(false);
  });

  it("does not release without a previous sample or focus", () => {
    expect(shouldReleaseFocusToFollow(null, "understand", "succeeded")).toBe(
      false,
    );
    expect(
      shouldReleaseFocusToFollow(
        { nodeId: "understand", status: "running" },
        null,
        "succeeded",
      ),
    ).toBe(false);
    expect(
      shouldReleaseFocusToFollow(
        { nodeId: "understand", status: "running" },
        "understand",
        undefined,
      ),
    ).toBe(false);
  });
});

describe("shouldAdvanceAutomaticConversation", () => {
  it("advances an open automatic session when its running node finishes", () => {
    expect(
      shouldAdvanceAutomaticConversation(
        { nodeId: "tests", status: "running" },
        "tests",
        "succeeded",
        false,
      ),
    ).toBe(true);
  });

  it("leaves interactive completion to its explicit action", () => {
    expect(
      shouldAdvanceAutomaticConversation(
        { nodeId: "quality", status: "running" },
        "quality",
        "succeeded",
        true,
      ),
    ).toBe(false);
  });

  it("does not advance a historical session that was already terminal", () => {
    expect(
      shouldAdvanceAutomaticConversation(
        { nodeId: "tests", status: "succeeded" },
        "tests",
        "succeeded",
        false,
      ),
    ).toBe(false);
  });
});

describe("resolveTheaterFocus", () => {
  it("keeps an explicit focus when the node exists", () => {
    const run = baseRun();
    expect(resolveTheaterFocus(run, "quality")).toEqual({
      primaryId: "quality",
      activeIds: [],
    });
  });

  it("tracks a single running node", () => {
    const run = baseRun({
      nodeStates: {
        start: { status: "succeeded", finishedAt: "2026-08-01T12:00:01+08:00" },
        understand: {
          status: "running",
          startedAt: "2026-08-01T12:00:02+08:00",
        },
        quality: { status: "idle" },
        tests: { status: "idle" },
        review: { status: "idle" },
        output: { status: "idle" },
      },
    });
    expect(resolveTheaterFocus(run, null)).toEqual({
      primaryId: "understand",
      activeIds: ["understand"],
    });
  });

  it("lists all parallel actives and prefers latest started among running", () => {
    const run = baseRun({
      nodeStates: {
        start: { status: "succeeded", finishedAt: "a" },
        understand: { status: "succeeded", finishedAt: "b" },
        quality: { status: "succeeded", finishedAt: "c" },
        tests: { status: "running", startedAt: "2026-08-01T12:00:10+08:00" },
        review: { status: "running", startedAt: "2026-08-01T12:00:12+08:00" },
        output: { status: "idle" },
      },
    });
    expect(resolveTheaterFocus(run, null)).toEqual({
      primaryId: "review",
      activeIds: ["tests", "review"],
    });
  });

  it("orders parallel actives by path order even when the snapshot array is reversed", () => {
    const snapshot = normalizeWorkflowDefinition(
      createMockWorkflowFixture("zh-CN"),
    );
    const reversed = {
      ...snapshot,
      nodes: [...snapshot.nodes].reverse(),
    };
    const run = baseRun({
      definitionSnapshot: reversed,
      nodeStates: {
        start: { status: "succeeded", finishedAt: "a" },
        understand: { status: "succeeded", finishedAt: "b" },
        quality: { status: "succeeded", finishedAt: "c" },
        tests: { status: "running", startedAt: "2026-08-01T12:00:10+08:00" },
        review: { status: "running", startedAt: "2026-08-01T12:00:12+08:00" },
        output: { status: "idle" },
      },
    });
    expect(reversed.nodes.map((node) => node.id)[0]).toBe("output");
    expect(resolveTheaterFocus(run, null).activeIds).toEqual([
      "tests",
      "review",
    ]);
  });

  it("prefers awaiting_input over running when choosing primary", () => {
    const run = baseRun({
      status: "awaiting_input",
      nodeStates: {
        start: { status: "succeeded", finishedAt: "a" },
        understand: {
          status: "running",
          startedAt: "2026-08-01T12:00:20+08:00",
        },
        quality: {
          status: "awaiting_input",
          startedAt: "2026-08-01T12:00:11+08:00",
        },
        tests: { status: "idle" },
        review: { status: "idle" },
        output: { status: "idle" },
      },
    });
    expect(resolveTheaterFocus(run, null)).toEqual({
      primaryId: "quality",
      activeIds: ["understand", "quality"],
    });
  });

  it("falls back to the latest succeeded node when nothing is active", () => {
    const run = baseRun({
      status: "succeeded",
      nodeStates: {
        start: { status: "succeeded", finishedAt: "2026-08-01T12:00:01+08:00" },
        understand: {
          status: "succeeded",
          finishedAt: "2026-08-01T12:00:05+08:00",
        },
        quality: {
          status: "succeeded",
          finishedAt: "2026-08-01T12:00:03+08:00",
        },
        tests: { status: "idle" },
        review: { status: "idle" },
        output: { status: "idle" },
      },
    });
    expect(resolveTheaterFocus(run, null)).toEqual({
      primaryId: "understand",
      activeIds: [],
    });
  });

  it("keeps resolveFocusNodeId aligned with primaryId", () => {
    const run = baseRun({
      nodeStates: {
        start: { status: "idle" },
        understand: { status: "running", startedAt: "t1" },
        quality: { status: "idle" },
        tests: { status: "idle" },
        review: { status: "idle" },
        output: { status: "idle" },
      },
    });
    expect(resolveFocusNodeId(run, null)).toBe(
      resolveTheaterFocus(run, null).primaryId,
    );
  });
});

describe("resolveOverviewFocusedId", () => {
  it("suppresses Theater fallback selection on a terminal run with no pin", () => {
    const run = baseRun({
      status: "succeeded",
      nodeStates: {
        start: { status: "succeeded", finishedAt: "2026-08-01T12:00:01+08:00" },
        understand: {
          status: "succeeded",
          finishedAt: "2026-08-01T12:00:05+08:00",
        },
        quality: { status: "idle" },
        tests: { status: "idle" },
        review: { status: "idle" },
        output: { status: "idle" },
      },
    });
    expect(resolveTheaterFocus(run, null).primaryId).toBe("understand");
    expect(resolveOverviewFocusedId(run, null)).toBeNull();
  });

  it("keeps an explicit pin on a terminal run", () => {
    const run = baseRun({ status: "failed" });
    expect(resolveOverviewFocusedId(run, "quality")).toBe("quality");
  });

  it("still auto-follows while the run is live", () => {
    const run = baseRun({
      nodeStates: {
        start: { status: "succeeded", finishedAt: "a" },
        understand: { status: "running", startedAt: "b" },
        quality: { status: "idle" },
        tests: { status: "idle" },
        review: { status: "idle" },
        output: { status: "idle" },
      },
    });
    expect(resolveOverviewFocusedId(run, null)).toBe("understand");
  });
});

describe("resolveStageFocusNodeId", () => {
  it("prefers an open node session over path focus", () => {
    expect(resolveStageFocusNodeId("review", "understand")).toBe("review");
    expect(resolveStageFocusNodeId(null, "understand")).toBe("understand");
    expect(resolveStageFocusNodeId(null, null)).toBeNull();
  });
});

describe("shouldReleaseLivePinToFollow", () => {
  it("blocks live-pin release while a node session is open", () => {
    expect(
      shouldReleaseLivePinToFollow(
        "understand",
        { nodeId: "understand", status: "running" },
        "understand",
        "succeeded",
      ),
    ).toBe(false);
  });

  it("still releases a live pin when no session is open", () => {
    expect(
      shouldReleaseLivePinToFollow(
        null,
        { nodeId: "understand", status: "running" },
        "understand",
        "succeeded",
      ),
    ).toBe(true);
  });
});

describe("shouldStealFocusForArtifactReveal", () => {
  it("does not steal focus while a node session is open", () => {
    expect(
      shouldStealFocusForArtifactReveal({
        conversationNodeId: "review",
        stagePrimaryId: "review",
        artifactNodeId: "output",
      }),
    ).toBe(false);
  });

  it("does not steal when the stage is already on the producing act", () => {
    expect(
      shouldStealFocusForArtifactReveal({
        conversationNodeId: null,
        stagePrimaryId: "output",
        artifactNodeId: "output",
      }),
    ).toBe(false);
  });

  it("steals focus onto a different producing act when idle", () => {
    expect(
      shouldStealFocusForArtifactReveal({
        conversationNodeId: null,
        stagePrimaryId: "review",
        artifactNodeId: "output",
      }),
    ).toBe(true);
  });
});

describe("retry_waiting focus", () => {
  // A waiting row has no startedAt: the backend inserts it with `started_at` null.
  const WAITING = {
    status: "retry_waiting" as const,
    retryWait: {
      attempt: 2,
      maxAttempt: 4,
      retry: 1,
      maxRetries: 3,
      delayMs: 30_000,
      scheduledAt: 1_000,
      dueAt: 31_000,
    },
    autoRetry: { retry: 1, maxRetries: 3 },
  };

  it("keeps a waiting node among the active acts in path order", () => {
    const run = baseRun({
      nodeStates: {
        start: { status: "succeeded", finishedAt: "2026-08-01T12:00:01+08:00" },
        understand: {
          status: "succeeded",
          finishedAt: "2026-08-01T12:00:05+08:00",
        },
        quality: { status: "running", startedAt: "2026-08-01T12:00:06+08:00" },
        tests: WAITING,
        review: { status: "running", startedAt: "2026-08-01T12:00:07+08:00" },
        output: { status: "idle" },
      },
    });

    expect(resolveTheaterFocus(run, null)).toEqual({
      primaryId: "review",
      activeIds: ["quality", "tests", "review"],
    });
    // An explicit pin on the waiting node is kept and the active list is unchanged.
    expect(resolveTheaterFocus(run, "tests")).toEqual({
      primaryId: "tests",
      activeIds: ["quality", "tests", "review"],
    });
  });

  it("follows a lone waiting node instead of the last succeeded one", () => {
    const run = baseRun({
      nodeStates: {
        start: { status: "succeeded", finishedAt: "2026-08-01T12:00:01+08:00" },
        understand: {
          status: "succeeded",
          finishedAt: "2026-08-01T12:00:05+08:00",
        },
        quality: WAITING,
        tests: { status: "idle" },
        review: { status: "idle" },
        output: { status: "idle" },
      },
    });

    expect(resolveTheaterFocus(run, null)).toEqual({
      primaryId: "quality",
      activeIds: ["quality"],
    });
    expect(resolveOverviewFocusedId(run, null)).toBe("quality");
    expect(resolveFocusNodeId(run, null)).toBe("quality");
  });

  it("prefers a started running node over a waiting node in either path position", () => {
    const waitingFirst = baseRun({
      nodeStates: {
        start: { status: "succeeded", finishedAt: "a" },
        understand: { status: "succeeded", finishedAt: "b" },
        quality: WAITING,
        tests: { status: "running", startedAt: "2026-08-01T12:00:10+08:00" },
        review: { status: "idle" },
        output: { status: "idle" },
      },
    });
    expect(resolveTheaterFocus(waitingFirst, null)).toEqual({
      primaryId: "tests",
      activeIds: ["quality", "tests"],
    });

    const runningFirst = baseRun({
      nodeStates: {
        start: { status: "succeeded", finishedAt: "a" },
        understand: { status: "succeeded", finishedAt: "b" },
        quality: { status: "running", startedAt: "2026-08-01T12:00:10+08:00" },
        tests: WAITING,
        review: { status: "idle" },
        output: { status: "idle" },
      },
    });
    expect(resolveTheaterFocus(runningFirst, null)).toEqual({
      primaryId: "quality",
      activeIds: ["quality", "tests"],
    });
  });

  it("keeps a live pin through retry waits and releases it once the node ends", () => {
    expect(
      shouldReleaseFocusToFollow(
        { nodeId: "tests", status: "running" },
        "tests",
        "retry_waiting",
      ),
    ).toBe(false);
    expect(
      shouldReleaseFocusToFollow(
        { nodeId: "tests", status: "retry_waiting" },
        "tests",
        "running",
      ),
    ).toBe(false);
    expect(
      shouldReleaseFocusToFollow(
        { nodeId: "tests", status: "retry_waiting" },
        "tests",
        "failed",
      ),
    ).toBe(true);
    // A wait abandoned because the run ended leaves the row cancelled.
    expect(
      shouldReleaseFocusToFollow(
        { nodeId: "tests", status: "retry_waiting" },
        "tests",
        "cancelled",
      ),
    ).toBe(true);
    expect(
      shouldReleaseLivePinToFollow(
        null,
        { nodeId: "tests", status: "running" },
        "tests",
        "retry_waiting",
      ),
    ).toBe(false);
  });

  it("does not advance an open automatic session when its node starts waiting to retry", () => {
    expect(
      shouldAdvanceAutomaticConversation(
        { nodeId: "tests", status: "running" },
        "tests",
        "retry_waiting",
        false,
      ),
    ).toBe(false);
    expect(
      shouldAdvanceAutomaticConversation(
        { nodeId: "tests", status: "retry_waiting" },
        "tests",
        "failed",
        false,
      ),
    ).toBe(true);
  });

  it("advances a finished node's session to a waiting successor", () => {
    const run = baseRun({
      nodeStates: {
        start: { status: "succeeded", finishedAt: "a" },
        understand: { status: "succeeded", finishedAt: "b" },
        quality: WAITING,
        tests: { status: "idle" },
        review: { status: "idle" },
        output: { status: "idle" },
      },
    });

    expect(resolveCompletionAdvanceNodeId(run, "understand")).toBe("quality");
  });
});
