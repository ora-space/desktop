import { describe, expect, it } from "vitest";
import { createMockWorkflow as createMockWorkflowFixture } from "@ora/workflow-mock";
import {
  resolveFocusNodeId,
  resolveTheaterFocus,
  shouldReleaseFocusToFollow,
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
        "skipped",
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
