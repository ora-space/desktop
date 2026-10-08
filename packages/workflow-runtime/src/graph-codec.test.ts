import { describe, expect, it } from "vitest";
import {
  isoToWorkflowTimestamp,
  parseWorkflowGraph,
  parseWorkflowGraphWithReport,
  serializeWorkflowGraph,
  workflowTimestampToIso,
} from "./graph-codec";
import {
  WORKFLOW_NODE_KINDS,
  type WorkflowAgentRetryPolicy,
  type WorkflowDefinitionEdge,
  type WorkflowDefinitionNode,
} from "./types";

const node: WorkflowDefinitionNode = {
  id: "start",
  type: "workflow",
  position: { x: 12, y: 34 },
  data: { kind: "start", title: "Start", description: "Receives input" },
};

const edge: WorkflowDefinitionEdge = {
  id: "e1",
  source: "start",
  target: "agent-1",
  type: "workflow",
};

describe("graph envelope codec", () => {
  it("round-trips nodes, edges, annotations, viewport, and description", () => {
    const annotation = {
      id: "annotation-1",
      type: "annotation" as const,
      position: { x: 48, y: 96 },
      width: 240,
      height: 140,
      data: { text: "Review this branch", theme: "yellow" as const },
    };
    const graph = serializeWorkflowGraph({
      nodes: [node],
      edges: [edge],
      annotations: [annotation],
      globalVariables: [{ name: "sys.workflow_id", valueType: "string" }],
      viewport: { x: 32, y: 64, zoom: 1.5 },
      description: "A review flow",
    });

    expect(parseWorkflowGraph(graph)).toEqual({
      nodes: [node],
      edges: [edge],
      annotations: [annotation],
      globalVariables: [{ name: "sys.workflow_id", valueType: "string" }],
      viewport: { x: 32, y: 64, zoom: 1.5 },
      description: "A review flow",
    });
  });

  it("omits the description key when absent", () => {
    const graph = serializeWorkflowGraph({
      nodes: [node],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
    });

    expect(JSON.parse(graph)).not.toHaveProperty("description");
    expect(parseWorkflowGraph(graph)).not.toHaveProperty("description");
    expect(parseWorkflowGraph(graph).annotations).toEqual([]);
  });

  it("tolerates partial envelopes with missing arrays", () => {
    const graph = JSON.stringify({ viewport: { x: 5, y: 6, zoom: 1 } });

    expect(parseWorkflowGraph(graph)).toEqual({
      nodes: [],
      edges: [],
      viewport: { x: 5, y: 6, zoom: 1 },
      annotations: [],
      globalVariables: [],
    });
  });

  it("drops malformed editor annotations before they reach custom node rendering", () => {
    const graph = JSON.stringify({
      nodes: [node],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
      annotations: [
        {
          id: "annotation-1",
          type: "annotation",
          position: { x: 0, y: 0 },
          data: { text: "unsafe theme", theme: "unknown" },
        },
      ],
    });

    expect(parseWorkflowGraph(graph).annotations).toEqual([]);
  });

  it("falls back to defaults for invalid JSON and non-object envelopes", () => {
    expect(parseWorkflowGraph("not json")).toEqual({
      nodes: [],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
      annotations: [],
      globalVariables: [],
    });
    expect(parseWorkflowGraph("[1,2]")).toEqual({
      nodes: [],
      edges: [],
      viewport: { x: 0, y: 0, zoom: 1 },
      annotations: [],
      globalVariables: [],
    });
  });

  it("preserves unknown fields through the round-trip", () => {
    const graph = JSON.stringify({
      nodes: [node],
      edges: [edge],
      viewport: { x: 0, y: 0, zoom: 1 },
      customMetadata: { owner: "rhythm" },
    });

    expect(parseWorkflowGraph(graph)).toMatchObject({
      customMetadata: { owner: "rhythm" },
    });
  });

  it("upgrades legacy prompt and model nodes to the agent kind on parse", () => {
    const graph = JSON.stringify({
      nodes: [
        {
          ...node,
          data: { kind: "prompt", title: "理解改动", description: "LLM 推理" },
        },
        {
          ...node,
          id: "legacy-model",
          data: { kind: "model", title: "总结", description: "LLM 推理" },
        },
      ],
      edges: [edge],
      viewport: { x: 0, y: 0, zoom: 1 },
    });

    expect(parseWorkflowGraph(graph).nodes.map((item) => item.data)).toEqual([
      expect.objectContaining({ kind: "agent", title: "理解改动" }),
      expect.objectContaining({ kind: "agent", title: "总结" }),
    ]);
  });

  it("writes schema v2 and preserves executable Loop container metadata", () => {
    const loop: WorkflowDefinitionNode = {
      id: "loop-1",
      type: "workflow",
      position: { x: 200, y: 0 },
      data: {
        kind: "loop",
        title: "Refine",
        description: "",
        loopConfig: {
          maxIterations: 3,
          variables: [
            {
              name: "draft",
              valueType: "string",
              initial: { kind: "constant", value: "seed" },
              feedback: ["refine-agent", "output"],
            },
          ],
          until: {
            logic: "and",
            conditions: [
              {
                variableSelector: ["refine-agent", "output"],
                operator: "not_empty",
              },
            ],
          },
          outputs: [
            {
              name: "result",
              variableSelector: ["refine-agent", "output"],
            },
          ],
        },
      },
    };
    const child: WorkflowDefinitionNode = {
      id: "refine-agent",
      type: "workflow",
      parentId: loop.id,
      position: { x: 80, y: 100 },
      data: {
        kind: "agent",
        title: "Refine draft",
        description: "",
        containerId: loop.id,
      },
    };
    const input = {
      nodes: [node, loop, child],
      edges: [edge],
      viewport: { x: 0, y: 0, zoom: 1 },
      annotations: [],
      globalVariables: [],
    };

    const graph = serializeWorkflowGraph(input);

    expect(JSON.parse(graph)).toHaveProperty("schemaVersion", 2);
    expect(parseWorkflowGraph(graph)).toEqual({
      schemaVersion: 2,
      ...input,
      nodes: [node, { ...loop, initialWidth: 620, initialHeight: 340 }, child],
    });
  });

  it("loads legacy iteration geometry without dimensions and preserves its entry handle", () => {
    const graph = JSON.stringify({
      nodes: [
        {
          id: "iter",
          type: "workflow",
          position: { x: 40, y: 80 },
          data: { kind: "iteration", title: "Iteration", description: "" },
        },
        {
          id: "agent",
          type: "workflow",
          parentId: "iter",
          position: { x: 96, y: 160 },
          data: { kind: "agent", title: "Agent", description: "" },
        },
      ],
      edges: [
        {
          id: "entry",
          source: "iter",
          sourceHandle: "iteration-entry",
          target: "agent",
        },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
    });

    const parsed = parseWorkflowGraph(graph);

    const source = JSON.parse(graph);
    expect(parsed).toEqual({
      ...source,
      nodes: [
        { ...source.nodes[0], initialWidth: 560, initialHeight: 400 },
        source.nodes[1],
      ],
      annotations: [],
      globalVariables: [],
    });
  });
});

describe("graph node sanitization", () => {
  /** Builds a graph string from raw node records, the way a foreign package supplies them. */
  function rawGraph(nodes: unknown[], edges: unknown[] = []): string {
    return JSON.stringify({
      nodes,
      edges,
      viewport: { x: 0, y: 0, zoom: 1 },
      annotations: [],
      globalVariables: [],
    });
  }

  it("keeps every node kind the render catalog supports", () => {
    // Spelled out rather than derived from WORKFLOW_NODE_KINDS: the list under test is the
    // thing that can shrink by mistake, and a derived expectation would shrink with it.
    const renderableKinds = [
      "start",
      "agent",
      "condition",
      "aggregator",
      "tool",
      "junction",
      "human",
      "loop",
      "loopExit",
      "iteration",
      "subflow",
      "output",
    ];
    expect(WORKFLOW_NODE_KINDS).toEqual(renderableKinds);

    const graph = rawGraph(
      renderableKinds.map((kind) => ({
        id: `n-${kind}`,
        type: "workflow",
        position: { x: 0, y: 0 },
        data: { kind, title: kind, description: "" },
      })),
    );

    const result = parseWorkflowGraphWithReport(graph);

    expect(result.droppedNodeCount).toBe(0);
    expect(result.droppedNodeKinds).toEqual([]);
    expect(result.envelope.nodes.map((item) => item.data.kind)).toEqual(
      renderableKinds,
    );
  });

  it("drops a node whose kind this version cannot render and reports the kind", () => {
    const graph = rawGraph([
      node,
      {
        id: "router",
        type: "workflow",
        position: { x: 0, y: 0 },
        data: { kind: "router", title: "Router", description: "" },
      },
    ]);

    const result = parseWorkflowGraphWithReport(graph);

    expect(result.envelope.nodes).toEqual([node]);
    expect(result.droppedNodeCount).toBe(1);
    expect(result.droppedNodeKinds).toEqual(["router"]);
  });

  it("drops edges into a dropped node and keeps the edges between rendered nodes", () => {
    const graph = rawGraph(
      [
        node,
        {
          id: "router",
          type: "workflow",
          position: { x: 0, y: 0 },
          data: { kind: "router", title: "Router", description: "" },
        },
        {
          id: "agent-1",
          type: "workflow",
          position: { x: 0, y: 0 },
          data: { kind: "agent", title: "Agent", description: "" },
        },
      ],
      [
        { id: "into-router", source: "start", target: "router" },
        { id: "out-of-router", source: "router", target: "agent-1" },
        { id: "kept", source: "start", target: "agent-1" },
      ],
    );

    const result = parseWorkflowGraphWithReport(graph);

    expect(result.envelope.edges.map((item) => item.id)).toEqual(["kept"]);
  });

  it("drops malformed node records without failing the parse", () => {
    const graph = rawGraph([
      null,
      { id: "no-data" },
      { data: { kind: "agent" } },
      { id: "   ", data: { kind: "agent" } },
      {
        id: "ok",
        type: "workflow",
        position: { x: 0, y: 0 },
        data: { kind: "agent", title: "Agent", description: "" },
      },
    ]);

    const result = parseWorkflowGraphWithReport(graph);

    expect(result.envelope.nodes.map((item) => item.id)).toEqual(["ok"]);
    expect(result.droppedNodeCount).toBe(4);
    expect(result.droppedNodeKinds).toEqual([]);
  });

  it("upgrades legacy kinds before deciding whether a node is renderable", () => {
    const graph = rawGraph([
      {
        id: "legacy",
        type: "workflow",
        position: { x: 0, y: 0 },
        data: { kind: "prompt", title: "Legacy", description: "" },
      },
    ]);

    const result = parseWorkflowGraphWithReport(graph);

    expect(result.droppedNodeCount).toBe(0);
    expect(result.envelope.nodes[0]?.data.kind).toBe("agent");
  });

  it("reports nothing dropped when the graph cannot be parsed at all", () => {
    const result = parseWorkflowGraphWithReport("not json");

    expect(result).toEqual({
      envelope: {
        nodes: [],
        edges: [],
        viewport: { x: 0, y: 0, zoom: 1 },
        annotations: [],
        globalVariables: [],
      },
      droppedNodeCount: 0,
      droppedNodeKinds: [],
    });
  });
});

describe("workflow timestamp projection", () => {
  it("converts epoch millis to an ISO string", () => {
    expect(workflowTimestampToIso(0)).toBe("1970-01-01T00:00:00.000Z");
  });

  it("round-trips an ISO string through the epoch-millis projection", () => {
    const iso = "2026-08-05T08:00:00.000Z";
    expect(workflowTimestampToIso(isoToWorkflowTimestamp(iso))).toBe(iso);
  });
});

// Saved drafts, published snapshots, and exported graphs share this envelope codec.
it("preserves canonical MCP IDs and disabled bindings across graph round trips", () => {
  const agent: WorkflowDefinitionNode = {
    id: "agent-1",
    type: "workflow",
    position: { x: 0, y: 0 },
    data: {
      kind: "agent",
      title: "Agent",
      description: "",
      agentConfig: {
        schemaVersion: 3,
        executor: { agentCli: "official/agent", modelId: "model" },
        roleId: "",
        skills: [],
        prompt: "",
        mcps: [
          { mcpId: "official/tools", enabled: true },
          { mcpId: "local/tools", enabled: false },
        ],
      },
    },
  };
  const input = {
    nodes: [agent],
    edges: [],
    viewport: { x: 0, y: 0, zoom: 1 },
    annotations: [],
    globalVariables: [],
  };
  expect(parseWorkflowGraph(serializeWorkflowGraph(input))).toEqual(input);
});

// Absent retry means "default policy" to the engine, so the codec must neither add nor drop it.
it("preserves agent retry settings and their absence across graph round trips", () => {
  const agent = (
    id: string,
    retry?: WorkflowAgentRetryPolicy,
  ): WorkflowDefinitionNode => ({
    id,
    type: "workflow",
    position: { x: 0, y: 0 },
    data: {
      kind: "agent",
      title: id,
      description: "",
      agentConfig: {
        schemaVersion: 3,
        executor: { agentCli: "official/agent", modelId: "model" },
        roleId: "",
        skills: [],
        mcps: [],
        prompt: "",
        ...(retry === undefined ? {} : { retry }),
      },
    },
  });
  const input = {
    nodes: [
      agent("default"),
      agent("tuned", { enabled: true, maxRetries: 5, initialDelaySeconds: 0 }),
      agent("off", { enabled: false, maxRetries: 2, initialDelaySeconds: 10 }),
    ],
    edges: [],
    viewport: { x: 0, y: 0, zoom: 1 },
    annotations: [],
    globalVariables: [],
  };

  const parsed = parseWorkflowGraph(serializeWorkflowGraph(input));

  expect(parsed).toEqual(input);
  expect(parsed.nodes[0]?.data.agentConfig).not.toHaveProperty("retry");
});

describe("graph codec aggregator round trip", () => {
  it("preserves the aggregator selector order and config through save and reload", () => {
    const aggregator: WorkflowDefinitionNode = {
      id: "agg",
      type: "workflow",
      position: { x: 0, y: 0 },
      data: {
        kind: "aggregator",
        title: "Agg",
        description: "",
        aggregatorConfig: {
          variables: [
            ["b", "output"],
            ["a", "output"],
          ],
        },
      },
    };
    const agentA: WorkflowDefinitionNode = {
      id: "a",
      type: "workflow",
      position: { x: 0, y: 0 },
      data: {
        kind: "agent",
        title: "A",
        description: "",
        agentConfig: {
          schemaVersion: 3,
          executor: { agentCli: "c", modelId: "m" },
          roleId: "",
          skills: [],
          mcps: [],
          prompt: "a",
        },
      },
    };
    const agentB: WorkflowDefinitionNode = {
      ...agentA,
      id: "b",
      data: { ...agentA.data, title: "B" },
    };
    const serialized = serializeWorkflowGraph({
      nodes: [aggregator, agentA, agentB],
      edges: [
        { id: "e1", source: "a", target: "agg" },
        { id: "e2", source: "b", target: "agg" },
      ],
      viewport: { x: 0, y: 0, zoom: 1 },
    });
    const parsed = parseWorkflowGraph(serialized);
    const reserialized = serializeWorkflowGraph({
      nodes: parsed.nodes,
      edges: parsed.edges,
      viewport: parsed.viewport,
    });
    expect(parseWorkflowGraph(reserialized).nodes[0]?.data).toEqual(
      aggregator.data,
    );
    const config = (
      parseWorkflowGraph(reserialized).nodes[0]?.data as {
        aggregatorConfig?: { variables: string[][] };
      }
    ).aggregatorConfig;
    expect(config?.variables).toEqual([
      ["b", "output"],
      ["a", "output"],
    ]);
  });
});
