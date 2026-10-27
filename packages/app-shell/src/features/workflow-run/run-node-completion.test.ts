import { describe, expect, it } from "vitest";
import type {
  GraphWorkflowRun,
  WorkflowNodeCompletion,
} from "@ora/workflow-runtime";
import {
  directSuccessorNodeTitles,
  isAwaitingConfirmation,
} from "./run-node-completion";

function node(id: string, title: string) {
  return {
    id,
    type: "workflow" as const,
    position: { x: 0, y: 0 },
    data: { kind: "agent" as const, title, description: "" },
  };
}

function runWithEdges(
  edges: Array<{ id: string; source: string; target: string }>,
): GraphWorkflowRun {
  return {
    id: "run-1",
    projectId: "project-1",
    definitionId: "workflow-1",
    definitionSnapshot: {
      id: "workflow-1",
      name: "Workflow",
      description: "",
      updatedAt: "2026-10-22T00:00:00+08:00",
      viewport: { x: 0, y: 0, zoom: 1 },
      nodes: [node("a", "Step A"), node("b", "Step B"), node("c", "Step C")],
      edges: edges.map((edge) => ({ ...edge, type: "workflow" })),
    },
    name: "Run",
    status: "running",
    nodeStates: {},
    openHitls: [],
    createdAt: "2026-10-22T00:00:00+08:00",
    updatedAt: "2026-10-22T00:00:00+08:00",
  };
}

describe("directSuccessorNodeTitles", () => {
  it("names every direct successor once and keeps the last step empty", () => {
    const titles = directSuccessorNodeTitles(
      runWithEdges([
        { id: "edge-1", source: "a", target: "b" },
        { id: "edge-2", source: "a", target: "c" },
        // Duplicate edges (retry loops or imported graphs) must not repeat the title.
        { id: "edge-3", source: "a", target: "c" },
      ]),
    );

    expect(titles).toEqual({
      a: ["Step B", "Step C"],
    });
    expect(titles.b).toBeUndefined();
  });
});

describe("isAwaitingConfirmation", () => {
  function completion(
    overrides: Partial<WorkflowNodeCompletion>,
  ): WorkflowNodeCompletion {
    return {
      blockers: [],
      warnings: [],
      provisional: false,
      ready: false,
      ...overrides,
    };
  }

  it("recognizes the unconfirmed opening turn and nothing else", () => {
    expect(isAwaitingConfirmation(undefined)).toBe(false);
    expect(isAwaitingConfirmation(completion({ ready: true }))).toBe(false);
    expect(
      isAwaitingConfirmation(
        completion({
          provisional: true,
          blockers: [{ kind: "turn_unknown", params: {} }],
        }),
      ),
    ).toBe(false);
    expect(
      isAwaitingConfirmation(
        completion({ blockers: [{ kind: "turn_stopped", params: {} }] }),
      ),
    ).toBe(false);
    expect(
      isAwaitingConfirmation(
        completion({
          blockers: [
            { kind: "turn_unconfirmed", params: {} },
            { kind: "turn_stopped", params: {} },
          ],
        }),
      ),
    ).toBe(false);
    expect(
      isAwaitingConfirmation(
        completion({ blockers: [{ kind: "turn_unconfirmed", params: {} }] }),
      ),
    ).toBe(true);
  });
});
