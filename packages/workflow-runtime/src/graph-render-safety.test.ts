import { describe, expect, it } from "vitest";
import {
  parseWorkflowGraph,
  parseWorkflowGraphWithReport,
} from "./graph-codec";

describe("untrusted snapshot render geometry", () => {
  it("grows authored ancestor frames when a descendant needs recovered geometry", () => {
    const nodes = parseWorkflowGraph(
      JSON.stringify({
        nodes: [
          {
            id: "outer",
            position: { x: 20, y: 20 },
            initialWidth: 620,
            initialHeight: 300,
            data: { kind: "loop" },
          },
          {
            id: "inner",
            position: { x: 40, y: 140 },
            data: { kind: "loop", containerId: "outer" },
          },
          ...["a", "b", "c"].map((id) => ({
            id,
            data: { kind: "agent", containerId: "inner" },
          })),
        ],
      }),
    ).nodes;
    expect(
      nodes
        .filter((node) => node.data.kind === "loop")
        .map(({ id, position, initialWidth, initialHeight }) => ({
          id,
          position,
          initialWidth,
          initialHeight,
        })),
    ).toEqual([
      {
        id: "outer",
        position: { x: 20, y: 20 },
        initialWidth: 1040,
        initialHeight: 560,
      },
      {
        id: "inner",
        position: { x: 40, y: 140 },
        initialWidth: 960,
        initialHeight: 380,
      },
    ]);
  });

  it("fits missing child geometry inside its container without moving authored positions", () => {
    const graph = JSON.stringify({
      nodes: [
        { id: "loop", data: { kind: "loop", title: "Loop" } },
        { id: "start", data: { kind: "start", containerId: "loop" } },
        { id: "agent", data: { kind: "agent", containerId: "loop" } },
        { id: "output", data: { kind: "output", containerId: "loop" } },
        { id: "next", data: { kind: "output" } },
        {
          id: "authored",
          position: { x: 80, y: 600 },
          data: { kind: "agent", containerId: "loop" },
        },
      ],
    });
    const nodes = parseWorkflowGraph(graph).nodes;
    expect(
      nodes.map(({ id, parentId, position, initialWidth, initialHeight }) => ({
        id,
        parentId,
        position,
        initialWidth,
        initialHeight,
      })),
    ).toEqual([
      {
        id: "loop",
        parentId: undefined,
        position: { x: 0, y: 0 },
        initialWidth: 1320,
        initialHeight: 840,
      },
      {
        id: "start",
        parentId: "loop",
        position: { x: 400, y: 140 },
        initialWidth: undefined,
        initialHeight: undefined,
      },
      {
        id: "agent",
        parentId: "loop",
        position: { x: 720, y: 140 },
        initialWidth: undefined,
        initialHeight: undefined,
      },
      {
        id: "output",
        parentId: "loop",
        position: { x: 1040, y: 140 },
        initialWidth: undefined,
        initialHeight: undefined,
      },
      {
        id: "next",
        parentId: undefined,
        position: { x: 1400, y: 0 },
        initialWidth: undefined,
        initialHeight: undefined,
      },
      {
        id: "authored",
        parentId: "loop",
        position: { x: 80, y: 600 },
        initialWidth: undefined,
        initialHeight: undefined,
      },
    ]);
    expect(parseWorkflowGraph(graph).nodes).toEqual(nodes);
  });

  it("supplies finite positions and unique edge IDs while retaining executable data", () => {
    const config = { schemaVersion: 3, prompt: "inspect", skills: [null] };
    const graph = JSON.stringify({
      nodes: [
        { id: "start", data: { kind: "start", title: {}, description: null } },
        {
          id: "agent",
          position: { x: "bad", y: 0 },
          data: {
            kind: "agent",
            title: "Agent",
            description: "",
            agentConfig: config,
          },
        },
      ],
      edges: [
        null,
        false,
        {},
        { source: "start", target: "agent" },
        { id: "workflow-edge-0", source: "start", target: "agent" },
        { id: "workflow-edge-0", source: "start", target: "agent" },
      ],
      viewport: { x: 0, y: 0, zoom: -1 },
      description: {},
    });
    const parsed = parseWorkflowGraph(graph);
    expect(parsed).toEqual({
      nodes: [
        {
          id: "start",
          type: "workflow",
          position: { x: 0, y: 0 },
          data: { kind: "start", title: "start", description: "" },
        },
        {
          id: "agent",
          type: "workflow",
          position: { x: 320, y: 0 },
          data: {
            kind: "agent",
            title: "Agent",
            description: "",
            agentConfig: config,
          },
        },
      ],
      edges: [
        { id: "workflow-edge-0-1", source: "start", target: "agent" },
        { id: "workflow-edge-0", source: "start", target: "agent" },
        { id: "workflow-edge-2", source: "start", target: "agent" },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
      annotations: [],
      globalVariables: [],
    });
    expect(parseWorkflowGraph(graph)).toEqual(parsed);
    expect(JSON.parse(graph).nodes[1].data.agentConfig).toEqual(config);
  });

  it("reports duplicate nodes without removing the first node's edges", () => {
    const node = {
      id: "start",
      data: { kind: "start", title: "Start", description: "" },
    };
    const parsed = parseWorkflowGraphWithReport(
      JSON.stringify({
        nodes: [node, node],
        edges: [{ source: "start", target: "start" }],
      }),
    );
    expect(parsed.droppedNodeCount).toBe(1);
    expect(parsed.envelope.nodes).toHaveLength(1);
    expect(parsed.envelope.edges).toEqual([
      { source: "start", target: "start", id: "workflow-edge-0" },
    ]);
  });

  it("retains edges when a usable record follows a dropped record with the same ID", () => {
    const parsed = parseWorkflowGraphWithReport(
      JSON.stringify({
        nodes: [
          { id: "start", data: { kind: "unknown" } },
          { id: "start", data: { kind: "start" } },
        ],
        edges: [{ source: "start", target: "start" }],
      }),
    );
    expect({
      dropped: parsed.droppedNodeCount,
      edges: parsed.envelope.edges,
    }).toEqual({
      dropped: 1,
      edges: [{ source: "start", target: "start", id: "workflow-edge-0" }],
    });
  });

  it("detaches visual parents that refer to ordinary cards instead of container frames", () => {
    const parsed = parseWorkflowGraph(
      JSON.stringify({
        nodes: [
          { id: "agent", data: { kind: "agent" } },
          {
            id: "child",
            parentId: "agent",
            data: { kind: "output", containerId: "agent" },
          },
        ],
      }),
    );
    expect(
      parsed.nodes.map(({ id, parentId, data }) => ({
        id,
        parentId,
        containerId: data.containerId,
      })),
    ).toEqual([
      { id: "agent", parentId: undefined, containerId: undefined },
      { id: "child", parentId: undefined, containerId: "agent" },
    ]);
  });

  it("detaches cyclic and dangling visual parents while preserving domain ownership", () => {
    const nodes = [
      {
        id: "a",
        parentId: "b",
        data: { kind: "loop", title: "A", containerId: "b" },
      },
      {
        id: "b",
        parentId: "a",
        data: { kind: "loop", title: "B", containerId: "a" },
      },
      {
        id: "orphan",
        parentId: "absent",
        data: { kind: "agent", title: "Orphan", containerId: "absent" },
      },
    ];
    const parsed = parseWorkflowGraph(JSON.stringify({ nodes }));
    expect(
      parsed.nodes.map(({ id, parentId, data }) => ({
        id,
        parentId,
        containerId: data.containerId,
      })),
    ).toEqual([
      { id: "a", parentId: undefined, containerId: "b" },
      { id: "b", parentId: "a", containerId: "a" },
      { id: "orphan", parentId: undefined, containerId: "absent" },
    ]);
  });
});
