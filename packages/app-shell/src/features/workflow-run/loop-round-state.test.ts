import { describe, expect, it } from "vitest";
import type { GraphWorkflowRun } from "@ora/workflow-runtime";
import {
  loopRoundOutcome,
  projectLoopRoundNodeStates,
  selectedLoopRound,
} from "./loop-round-state";
import { resolveTheaterFocus } from "./run-focus";

function createRun(): GraphWorkflowRun {
  return {
    id: "run",
    projectId: "project",
    definitionId: "definition",
    name: "Loop run",
    status: "running",
    definitionSnapshot: {
      id: "definition",
      name: "Loop workflow",
      description: "Exercises Loop round state projection.",
      updatedAt: "2026-09-18T08:00:00.000Z",
      viewport: { x: 0, y: 0, zoom: 1 },
      nodes: [
        {
          id: "loop",
          type: "workflow",
          position: { x: 0, y: 0 },
          data: {
            kind: "loop",
            title: "Loop",
            description: "Iterate over the draft.",
          },
        },
        {
          id: "writer",
          type: "workflow",
          position: { x: 100, y: 0 },
          data: {
            kind: "agent",
            title: "Writer",
            description: "Write the draft.",
            containerId: "loop",
          },
        },
        {
          id: "reviewer",
          type: "workflow",
          position: { x: 200, y: 0 },
          data: {
            kind: "agent",
            title: "Reviewer",
            description: "Review the draft.",
            containerId: "loop",
          },
        },
        {
          id: "outside",
          type: "workflow",
          position: { x: 300, y: 0 },
          data: {
            kind: "output",
            title: "Output",
            description: "Return the result.",
          },
        },
      ],
      edges: [],
    },
    nodeStates: {
      loop: {
        status: "running",
        startedAt: "2026-09-18T08:00:00.000Z",
      },
      outside: { status: "succeeded" },
    },
    rounds: [
      {
        id: "round-1",
        parentLoopNodeRunId: "loop-run",
        parentLoopNodeId: "loop",
        roundIndex: 1,
        status: "succeeded",
        nodeStates: {
          writer: {
            status: "succeeded",
            sessionId: "writer-round-1",
            output: { summary: "first draft" },
          },
          reviewer: { status: "succeeded" },
        },
        createdAt: "2026-09-18T08:00:00.000Z",
        updatedAt: "2026-09-18T08:01:00.000Z",
      },
      {
        id: "round-2",
        parentLoopNodeRunId: "loop-run",
        parentLoopNodeId: "loop",
        roundIndex: 2,
        status: "running",
        nodeStates: {
          writer: {
            status: "running",
            sessionId: "writer-round-2",
            startedAt: "2026-09-18T08:02:00.000Z",
          },
          reviewer: { status: "idle" },
        },
        createdAt: "2026-09-18T08:02:00.000Z",
        updatedAt: "2026-09-18T08:03:00.000Z",
      },
    ],
    openHitls: [],
    createdAt: "2026-09-18T08:00:00.000Z",
    updatedAt: "2026-09-18T08:03:00.000Z",
  };
}

describe("Loop round state projection", () => {
  it("distinguishes budget exhaustion from a met condition only on the last round", () => {
    const run = createRun();
    run.nodeStates.loop = {
      nodeRunId: "loop-run",
      status: "succeeded",
      stopReason: "loop_max_iterations",
    };
    run.rounds![1].status = "succeeded";
    expect(run.rounds!.map((round) => loopRoundOutcome(run, round))).toEqual([
      "workflowRun.loopView.continue",
      "workflowRun.loopView.maxIterations",
    ]);
  });
  it("keeps a rebuilt container's stop reason off older rounds", () => {
    const run = createRun();
    run.nodeStates.loop = {
      nodeRunId: "rebuilt-loop",
      status: "succeeded",
      stopReason: "loop_exit",
    };
    run.rounds![1].status = "succeeded";
    const rebuilt = {
      ...run.rounds![0],
      id: "new-round",
      parentLoopNodeRunId: "rebuilt-loop",
    };
    run.rounds!.push(rebuilt);
    expect(run.rounds!.map((round) => loopRoundOutcome(run, round))).toEqual([
      "workflowRun.loopView.continue",
      "workflowRun.loopView.completed",
      "workflowRun.loopView.exit",
    ]);
  });
  it("never falls back to another round's session for an unexecuted member", () => {
    const run = createRun();
    run.nodeStates.reviewer = {
      status: "succeeded",
      sessionId: "stale",
      output: { summary: "stale result" },
    };
    delete run.rounds![0].nodeStates.reviewer;
    expect(projectLoopRoundNodeStates(run, {}).reviewer).toEqual({
      status: "inactive",
    });
    run.rounds = [];
    expect(projectLoopRoundNodeStates(run, {}).writer).toEqual({
      status: "inactive",
    });
  });
  it("uses the first round by default while preserving root-scope states", () => {
    const run = createRun();
    const nodeStates = projectLoopRoundNodeStates(run, {});

    expect(nodeStates).toEqual({
      ...run.nodeStates,
      ...run.rounds![0].nodeStates,
    });
    expect(resolveTheaterFocus({ ...run, nodeStates }, null).activeIds).toEqual(
      ["loop"],
    );
  });

  it("projects the explicitly selected historical round with its session and output", () => {
    const run = createRun();

    expect(projectLoopRoundNodeStates(run, { loop: "round-1" })).toEqual({
      loop: {
        status: "running",
        startedAt: "2026-09-18T08:00:00.000Z",
      },
      outside: { status: "succeeded" },
      writer: {
        status: "succeeded",
        sessionId: "writer-round-1",
        output: { summary: "first draft" },
      },
      reviewer: { status: "succeeded" },
    });
    expect(
      selectedLoopRound(run.rounds ?? [], "loop", { loop: "round-1" })?.id,
    ).toBe("round-1");
  });
});
