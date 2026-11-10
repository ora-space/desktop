import { describe, expect, it } from "vitest";
import type { Edge, Node } from "@xyflow/react";
import type { WorkflowNodeData } from "@ora/workflow-mock";
import {
  captureSelection,
  isTextEntryTarget,
  materializeCopy,
  WORKFLOW_DUPLICATE_STEP,
} from "./duplicate-selection";

function node(
  id: string,
  kind: WorkflowNodeData["kind"],
  extras: Partial<Node<WorkflowNodeData, "workflow">> & {
    data?: Partial<WorkflowNodeData>;
  } = {},
): Node<WorkflowNodeData, "workflow"> {
  const { data, ...rest } = extras;
  return {
    id,
    type: "workflow",
    position: { x: 100, y: 80 },
    data: {
      kind,
      title: id,
      description: "",
      ...data,
    },
    ...rest,
  };
}

describe("captureSelection", () => {
  it("keeps an edge only when both ends are selected", () => {
    const nodes = [
      node("start", "start", { selected: true, position: { x: 0, y: 0 } }),
      node("agent-1", "agent", { selected: true, position: { x: 200, y: 0 } }),
      node("agent-2", "agent", { position: { x: 400, y: 0 } }),
    ];
    const edges: Edge[] = [
      { id: "edge-1", source: "start", target: "agent-1" },
      { id: "edge-2", source: "agent-1", target: "agent-2" },
    ];
    const copy = captureSelection(nodes, edges);
    expect(copy.nodes.map((item) => item.id)).toEqual(["start", "agent-1"]);
    expect(copy.edges.map((edge) => edge.id)).toEqual(["edge-1"]);
  });

  it("includes the subgraph when the frame is selected and leaves a lone child in its parent", () => {
    const frame = node("iteration-1", "iteration", {
      selected: true,
      position: { x: 40, y: 40 },
    });
    const child = node("agent-1", "agent", {
      parentId: "iteration-1",
      position: { x: 16, y: 24 },
      data: {
        kind: "agent",
        title: "Child",
        description: "",
        containerId: "iteration-1",
      },
    });
    const onlyChild = captureSelection(
      [
        { ...child, selected: true },
        { ...frame, selected: false },
      ],
      [],
    );
    expect(onlyChild.nodes.map((item) => item.id)).toEqual(["agent-1"]);

    const withFrame = captureSelection(
      [frame, { ...child, selected: false }],
      [],
    );
    expect(withFrame.nodes.map((item) => item.id)).toEqual([
      "iteration-1",
      "agent-1",
    ]);
  });
});

describe("materializeCopy", () => {
  it("assigns new ids, copies config, and offsets by one grid step", () => {
    const original = node("agent-1", "agent", {
      data: {
        kind: "agent",
        title: "Agent",
        description: "keep",
        agentConfig: {
          schemaVersion: 3,
          executor: { agentCli: "codex", modelId: "m" },
          roleId: "",
          skills: [],
          mcps: [],
          prompt: "hello",
        },
      },
    });
    const copy = captureSelection([{ ...original, selected: true }], []);
    original.data.description = "changed";
    const created = materializeCopy(
      copy,
      ["agent-1"],
      { x: WORKFLOW_DUPLICATE_STEP, y: WORKFLOW_DUPLICATE_STEP },
      false,
    );
    expect(created.nodes).toHaveLength(1);
    expect(created.nodes[0]?.id).toBe("agent-2");
    expect(created.nodes[0]?.data.description).toBe("keep");
    expect(created.nodes[0]?.position).toEqual({
      x: 100 + WORKFLOW_DUPLICATE_STEP,
      y: 80 + WORKFLOW_DUPLICATE_STEP,
    });
    expect(created.nodes[0]?.selected).toBe(true);
  });

  it("remaps an internal edge and leaves a child inside the original parent", () => {
    const copy = captureSelection(
      [
        node("agent-1", "agent", {
          selected: true,
          position: { x: 10, y: 10 },
        }),
        node("agent-2", "agent", {
          selected: true,
          position: { x: 40, y: 10 },
        }),
        node("agent-3", "agent", {
          selected: true,
          parentId: "iteration-1",
          position: { x: 8, y: 12 },
          data: {
            kind: "agent",
            title: "In frame",
            description: "",
            containerId: "iteration-1",
          },
        }),
      ],
      [
        {
          id: "edge-9",
          source: "agent-1",
          target: "agent-2",
          sourceHandle: "ok",
        },
      ],
    );
    const created = materializeCopy(
      copy,
      ["agent-1", "agent-2", "agent-3", "edge-9"],
      { x: 20, y: 20 },
      false,
    );
    const edge = created.edges[0];
    expect(edge?.source).toBe("agent-4");
    expect(edge?.target).toBe("agent-5");
    expect(edge?.sourceHandle).toBe("ok");
    expect(edge?.id).not.toBe("edge-9");
    const child = created.nodes.find((item) => item.data.title === "In frame");
    expect(child?.parentId).toBe("iteration-1");
    expect(child?.data.containerId).toBe("iteration-1");
    expect(child?.position).toEqual({ x: 28, y: 32 });
  });

  it("copies a frame with its children and rewrites ids inside the subgraph", () => {
    const copy = captureSelection(
      [
        node("loop-1", "loop", {
          selected: true,
          position: { x: 0, y: 0 },
          data: {
            kind: "loop",
            title: "Loop",
            description: "",
            loopConfig: {
              maxIterations: 3,
              maxIterationsBehavior: "fail",
              variables: [
                {
                  name: "value",
                  valueType: "string",
                  initial: { kind: "constant", value: "" },
                  feedback: ["loop-1-agent", "output"],
                },
              ],
              until: {
                logic: "and",
                conditions: [
                  {
                    variableSelector: ["loop-1-agent", "output"],
                    operator: "not_empty",
                  },
                ],
              },
              outputs: [],
            },
          },
        }),
        node("loop-1-agent", "agent", {
          parentId: "loop-1",
          position: { x: 30, y: 40 },
          data: {
            kind: "agent",
            title: "Loop Agent",
            description: "",
            containerId: "loop-1",
            agentConfig: {
              schemaVersion: 3,
              executor: { agentCli: "codex", modelId: "m" },
              roleId: "",
              skills: [],
              mcps: [],
              prompt: "use {{#loop-1.value#}}",
            },
          },
        }),
      ],
      [{ id: "edge-1", source: "loop-1", target: "loop-1-agent" }],
    );
    const created = materializeCopy(copy, ["loop-1"], { x: 20, y: 20 }, true);
    const frame = created.nodes.find((item) => item.data.kind === "loop");
    const child = created.nodes.find((item) => item.data.kind === "agent");
    expect(frame?.id).toBe("loop-2");
    expect(frame?.position).toEqual({ x: 20, y: 20 });
    expect(child?.id).toBe("loop-2-agent");
    expect(child?.parentId).toBe("loop-2");
    expect(child?.position).toEqual({ x: 30, y: 40 });
    expect(child?.data.containerId).toBe("loop-2");
    expect(child?.data.agentConfig?.prompt).toBe("use {{#loop-2.value#}}");
    expect(frame?.data.loopConfig?.variables[0]?.feedback).toEqual([
      "loop-2-agent",
      "output",
    ]);
    expect(created.edges[0]).toMatchObject({
      source: "loop-2",
      target: "loop-2-agent",
    });
  });

  it("drops a graph start when the graph already has one", () => {
    const copy = captureSelection(
      [
        node("start", "start", { selected: true }),
        node("agent-1", "agent", { selected: true }),
      ],
      [{ id: "edge-1", source: "start", target: "agent-1" }],
    );
    const created = materializeCopy(
      copy,
      ["start", "agent-1"],
      { x: 20, y: 20 },
      true,
    );
    expect(created.nodes.map((item) => item.data.kind)).toEqual(["agent"]);
    expect(created.edges).toEqual([]);
  });
});

describe("isTextEntryTarget", () => {
  it("ignores graph shortcuts while a text control is focused", () => {
    const input = document.createElement("input");
    const area = document.createElement("textarea");
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    const canvas = document.createElement("div");
    expect(isTextEntryTarget(input)).toBe(true);
    expect(isTextEntryTarget(area)).toBe(true);
    const token = document.createElement("span");
    token.setAttribute("contenteditable", "false");
    editable.append(token);
    expect(isTextEntryTarget(editable)).toBe(true);
    expect(isTextEntryTarget(token)).toBe(true);
    expect(isTextEntryTarget(canvas)).toBe(false);
  });
});
