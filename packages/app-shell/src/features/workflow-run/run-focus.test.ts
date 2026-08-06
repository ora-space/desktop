import { describe, expect, it } from "vitest";
import { createMockWorkflow as createMockWorkflowFixture } from "@ora/workflow-mock";
import {
  resolveFocusNodeId,
  resolveOverviewFocusedId,
  resolveStageFocusNodeId,
  resolveTheaterFocus,
  shouldReleaseFocusToFollow,
  shouldReleaseLivePinToFollow,
  shouldStealFocusForArtifactReveal,
} from "./run-focus";
import {
  normalizeWorkflowDefinition,
  type GraphWorkflowRun,
} from "@ora/workflow-runtime";

function baseRun(
  overrides: Partial<GraphWorkflowRun> = {},
): GraphWorkflowRun {
  const snapshot = normalizeWorkflowDefinition(createMockWorkflowFixture("zh-CN"));
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
    totals: {},
    createdAt: "2026-08-01T12:00:00+08:00",
    updatedAt: "2026-08-01T12:00:00+08:00",
    ...overrides,
  };
}

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
    expect(
      shouldReleaseFocusToFollow(null, "understand", "succeeded"),
    ).toBe(false);
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
        understand: { status: "running", startedAt: "2026-08-01T12:00:02+08:00" },
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

  it("prefers awaiting_input over running when choosing primary", () => {
    const run = baseRun({
      status: "awaiting_input",
      nodeStates: {
        start: { status: "succeeded", finishedAt: "a" },
        understand: { status: "running", startedAt: "2026-08-01T12:00:20+08:00" },
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
        understand: { status: "succeeded", finishedAt: "2026-08-01T12:00:05+08:00" },
        quality: { status: "succeeded", finishedAt: "2026-08-01T12:00:03+08:00" },
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
        understand: { status: "succeeded", finishedAt: "2026-08-01T12:00:05+08:00" },
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
