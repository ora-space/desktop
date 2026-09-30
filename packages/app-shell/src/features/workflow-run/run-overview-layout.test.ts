import { describe, expect, it } from "vitest";
import type { WorkflowDefinition } from "@ora/workflow-runtime";
import { createRunOverviewNodes } from "./run-overview-layout";

describe("createRunOverviewNodes", () => {
  it("renders Loop children inside the persisted container geometry", () => {
    const definition: WorkflowDefinition = {
      id: "workflow",
      name: "Loop workflow",
      description: "",
      updatedAt: "2026-09-18T10:00:00.000Z",
      viewport: { x: 0, y: 0, zoom: 1 },
      nodes: [
        {
          id: "agent",
          type: "workflow",
          position: { x: 360, y: 145 },
          data: {
            kind: "agent",
            title: "Agent",
            description: "",
            containerId: "loop",
          },
        },
        {
          id: "loop",
          type: "workflow",
          position: { x: 200, y: 100 },
          initialWidth: 820,
          initialHeight: 440,
          data: { kind: "loop", title: "Loop", description: "" },
        },
      ],
      edges: [],
    };

    expect(
      createRunOverviewNodes(definition, {
        loop: { status: "running" },
        agent: { status: "succeeded" },
      }),
    ).toEqual([
      {
        id: "loop",
        type: "workflow",
        position: { x: 200, y: 100 },
        initialWidth: 820,
        initialHeight: 440,
        selectable: true,
        draggable: false,
        connectable: false,
        deletable: false,
        extent: undefined,
        expandParent: false,
        zIndex: 0,
        data: {
          kind: "loop",
          title: "Loop",
          description: "",
          runStatus: "running",
        },
      },
      {
        id: "agent",
        type: "workflow",
        position: { x: 360, y: 145 },
        parentId: "loop",
        extent: "parent",
        expandParent: false,
        selectable: true,
        draggable: false,
        connectable: false,
        deletable: false,
        zIndex: 1,
        data: {
          kind: "agent",
          title: "Agent",
          description: "",
          containerId: "loop",
          runStatus: "succeeded",
        },
      },
    ]);
  });
});
