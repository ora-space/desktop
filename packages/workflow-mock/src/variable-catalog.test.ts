import { describe, expect, it } from "vitest";
import type { Edge, Node } from "@xyflow/react";
import { deriveWorkflowVariableCatalog } from "./variable-catalog";
import { createMockWorkflowLoopGroup } from "./node-factory";
import type { WorkflowNodeData } from "./node-data";

function node(
  id: string,
  data: WorkflowNodeData,
  parentId?: string,
): Node<WorkflowNodeData, "workflow"> {
  return {
    id,
    type: "workflow",
    position: { x: 0, y: 0 },
    data,
    ...(parentId === undefined ? {} : { parentId }),
  };
}

function iterationData(
  iterator: string[],
  collect: string[],
): WorkflowNodeData {
  return {
    kind: "iteration",
    title: "Iterate",
    description: "",
    iterationConfig: {
      iteratorSelector: iterator,
      collectSelector: collect,
      errorStrategy: "fail",
      maxIterations: 10,
    },
  };
}

const START = node("start", {
  kind: "start",
  title: "Start",
  description: "",
  inputVariables: [
    { name: "prs", valueType: "array[object]" },
    { name: "title", valueType: "string" },
  ],
});
const ITER = node("iter", iterationData(["start", "prs"], ["fix", "output"]));
const FIX = node(
  "fix",
  {
    kind: "agent",
    title: "Fix",
    description: "",
    agentConfig: {
      schemaVersion: 3,
      executor: { agentCli: "c", modelId: "m" },
      roleId: "",
      skills: [],
      mcps: [],
      prompt: "fix",
    },
  },
  "iter",
);
const OUT = node("out", { kind: "output", title: "Out", description: "" });

const EDGES: Edge[] = [
  { id: "e1", source: "start", target: "iter" },
  { id: "e2", source: "iter", target: "fix" },
  { id: "e3", source: "fix", target: "out", sourceHandle: null },
  { id: "e4", source: "iter", target: "out" },
];

describe("deriveWorkflowVariableCatalog iteration scope rules", () => {
  it("exposes item and index to region members but never the exposed results", () => {
    const entries = deriveWorkflowVariableCatalog(
      [START, ITER, FIX, OUT],
      EDGES,
      "fix",
    );
    const selectors = entries.map((entry) => entry.selector.join("."));
    expect(selectors).toContain("iter.item");
    expect(selectors).toContain("iter.index");
    // Upstream-of-the-iteration variables stay visible inside the region.
    expect(selectors).toContain("start.prs");
    expect(selectors).toContain("start.title");
    expect(selectors).not.toContain("iter.output");
    expect(selectors).not.toContain("iter.entries");
    expect(selectors).not.toContain("iter.failed_count");
    // The item element type follows the iterator's declared array type.
    const item = entries.find(
      (entry) => entry.selector.join(".") === "iter.item",
    );
    expect(item?.valueType).toBe("object");
  });

  it("exposes the three fixed-type results to outer consumers only", () => {
    const entries = deriveWorkflowVariableCatalog(
      [START, ITER, FIX, OUT],
      EDGES,
      "out",
    );
    const bySelector = new Map(
      entries.map((entry) => [entry.selector.join("."), entry]),
    );
    expect(bySelector.get("iter.output")?.valueType).toBe("array[string]");
    expect(bySelector.get("iter.entries")?.valueType).toBe("array[object]");
    expect(bySelector.get("iter.failed_count")?.valueType).toBe("number");
    // Region members' products stay region-private.
    expect(bySelector.has("fix.output")).toBe(false);
    // Round bindings are region-private.
    expect(bySelector.has("iter.item")).toBe(false);
  });

  it("keeps the exposed types identical across error strategies", () => {
    for (const errorStrategy of ["fail", "continue"] as const) {
      const iter = node(
        "iter",
        iterationData(["start", "prs"], ["fix", "output"]),
      );
      iter.data.iterationConfig = {
        ...iter.data.iterationConfig!,
        errorStrategy,
      };
      const entries = deriveWorkflowVariableCatalog(
        [START, iter, FIX, OUT],
        EDGES,
        "out",
      );
      const bySelector = new Map(
        entries.map((entry) => [entry.selector.join("."), entry]),
      );
      expect(bySelector.get("iter.output")?.valueType).toBe("array[string]");
      expect(bySelector.get("iter.entries")?.valueType).toBe("array[object]");
      expect(bySelector.get("iter.failed_count")?.valueType).toBe("number");
    }
  });

  it("resolves iterator item types from the complete global selector", () => {
    const iter = node(
      "iter",
      iterationData(["vars", "rows"], ["fix", "output"]),
    );
    const entries = deriveWorkflowVariableCatalog(
      [
        node("start", {
          kind: "start",
          title: "Start",
          description: "",
          inputVariables: [{ name: "rows", valueType: "array[number]" }],
        }),
        iter,
        FIX,
        OUT,
      ],
      EDGES,
      "fix",
      [{ name: "vars.rows", valueType: "array[string]" }],
    );
    expect(
      entries.find((entry) => entry.selector.join(".") === "iter.item")
        ?.valueType,
    ).toBe("string");
  });
});

describe("deriveWorkflowVariableCatalog aggregator typing", () => {
  const AGENT = (id: string) =>
    node(id, {
      kind: "agent",
      title: id,
      description: "",
      agentConfig: {
        schemaVersion: 3,
        executor: { agentCli: "c", modelId: "m" },
        roleId: "",
        skills: [],
        mcps: [],
        prompt: "work",
      },
    });

  const aggregatorNode = (variables: string[][]) =>
    node("agg", {
      kind: "aggregator",
      title: "Agg",
      description: "",
      aggregatorConfig: { variables },
    });

  const graph = (agg: ReturnType<typeof aggregatorNode>) => [
    node("start", {
      kind: "start",
      title: "Start",
      description: "",
      inputVariables: [{ name: "limit", valueType: "number" }],
    }),
    AGENT("a"),
    AGENT("b"),
    agg,
    node("out", { kind: "output", title: "Out", description: "" }),
  ];

  const joinEdges: Edge[] = [
    { id: "e1", source: "start", target: "a" },
    { id: "e2", source: "start", target: "b" },
    { id: "e3", source: "a", target: "agg" },
    { id: "e4", source: "b", target: "agg" },
    { id: "e5", source: "agg", target: "out" },
  ];

  it("types the aggregator output as the candidates' common declared type", () => {
    const entries = deriveWorkflowVariableCatalog(
      graph(
        aggregatorNode([
          ["a", "output"],
          ["b", "output"],
        ]),
      ),
      joinEdges,
      "out",
    );
    const output = entries.find(
      (entry) => entry.selector.join(".") === "agg.output",
    );
    expect(output?.valueType).toBe("string");
  });

  it("derives mixed candidate types as any and keeps node outputs string", () => {
    const entries = deriveWorkflowVariableCatalog(
      graph(
        aggregatorNode([
          ["a", "output"],
          ["start", "limit"],
        ]),
      ),
      joinEdges,
      "out",
    );
    const bySelector = new Map(
      entries.map((entry) => [entry.selector.join("."), entry]),
    );
    expect(bySelector.get("agg.output")?.valueType).toBe("any");
  });

  it("derives an empty candidate list as any", () => {
    const entries = deriveWorkflowVariableCatalog(
      graph(aggregatorNode([])),
      joinEdges,
      "out",
    );
    expect(
      entries.find((entry) => entry.selector.join(".") === "agg.output")
        ?.valueType,
    ).toBe("any");
  });

  it("types chained aggregators from their upstream aggregator output", () => {
    const chained = node("agg2", {
      kind: "aggregator",
      title: "Agg2",
      description: "",
      aggregatorConfig: { variables: [["agg", "output"]] },
    });
    const entries = deriveWorkflowVariableCatalog(
      [
        ...graph(
          aggregatorNode([
            ["a", "output"],
            ["b", "output"],
          ]),
        ),
        chained,
        node("out2", { kind: "output", title: "Out2", description: "" }),
      ],
      [
        ...joinEdges,
        { id: "e6", source: "agg", target: "agg2" },
        { id: "e7", source: "agg2", target: "out2" },
      ],
      "out2",
    );
    expect(
      entries.find((entry) => entry.selector.join(".") === "agg2.output")
        ?.valueType,
    ).toBe("string");
  });
});

describe("deriveWorkflowVariableCatalog loop scope rules", () => {
  // The editor's own Loop group: a round Start and an Agent fed back as `value`,
  // exported as the named output `result`.
  const group = createMockWorkflowLoopGroup({
    sequence: 1,
    position: { x: 0, y: 0 },
    locale: "zh-CN",
  });
  const [loop, roundStart, loopAgent] = group.nodes as [
    Node<WorkflowNodeData, "workflow">,
    Node<WorkflowNodeData, "workflow">,
    Node<WorkflowNodeData, "workflow">,
  ];
  const loopStart = node("start", {
    kind: "start",
    title: "Start",
    description: "",
    inputVariables: [{ name: "topic", valueType: "string" }],
  });
  const after = node("after", {
    kind: "output",
    title: "After",
    description: "",
  });
  const nodes = [loopStart, loop, roundStart, loopAgent, after];
  const edges: Edge[] = [
    { id: "s-l", source: "start", target: loop.id },
    ...group.edges,
    { id: "l-a", source: loop.id, target: "after" },
  ];
  const selectors = (consumer: string): string[] =>
    deriveWorkflowVariableCatalog(nodes, edges, consumer, []).map((entry) =>
      entry.selector.join("."),
    );

  it("offers outer consumers the named outputs, never loop.output", () => {
    const visible = selectors("after");
    expect(visible).toContain(`${loop.id}.result`);
    expect(visible).not.toContain(`${loop.id}.output`);
    // Neither loop variables nor body products leak out of the Loop.
    expect(visible).not.toContain(`${loop.id}.value`);
    expect(visible).not.toContain(`${loopAgent.id}.output`);
    expect(visible).not.toContain(`${roundStart.id}.input`);
    expect(visible).toEqual(
      expect.arrayContaining(["start.input", "start.topic"]),
    );
  });

  it("types named outputs as the backend declares them", () => {
    const types = (names: string[]): string[] => {
      const named = node(loop.id, {
        ...loop.data,
        loopConfig: {
          ...loop.data.loopConfig!,
          outputs: names.map((name) => ({
            name,
            variableSelector: [loopAgent.id, "output"],
          })),
        },
      });
      return deriveWorkflowVariableCatalog(
        nodes.map((candidate) =>
          candidate.id === loop.id ? named : candidate,
        ),
        edges,
        "after",
        [],
      )
        .filter((entry) => entry.sourceNodeId === loop.id)
        .map((entry) => `${entry.variableName}:${entry.valueType}`);
    };
    // Every producing node declares `output` as a string first; a named output of that name
    // keeps the type.
    expect(types(["result", "structured_output", "output", " "])).toEqual([
      "result:any",
      "structured_output:any",
      "output:string",
    ]);
  });

  it("types an aggregator over Loop outputs as the runtime declares them", () => {
    const aggregate = (variable: string): string | undefined =>
      deriveWorkflowVariableCatalog(
        [
          ...nodes,
          node("agg", {
            kind: "aggregator",
            title: "Agg",
            description: "",
            aggregatorConfig: { variables: [[loop.id, variable]] },
          }),
          node("end", { kind: "output", title: "End", description: "" }),
        ],
        [
          ...edges,
          { id: "l-g", source: loop.id, target: "agg" },
          { id: "g-e", source: "agg", target: "end" },
        ],
        "end",
        [],
      ).find((entry) => entry.sourceNodeId === "agg")?.valueType;
    expect(aggregate("result")).toBe("any");
    expect(aggregate("structured_output")).toBe("any");
    expect(aggregate("output")).toBe("string");
  });

  // Importing a draft keeps `loopConfig` as written, and the editor derives the catalog with
  // and without a selected node before the author can repair it.
  it.each([
    ["an object for a list", {}],
    ["a string for a list", "result"],
    ["null for a list", null],
    ["no list", undefined],
    ["a null entry", [null]],
    ["an entry without a name", [{}]],
    ["an entry whose name is not text", [{ name: 1 }]],
    ["an entry with a blank name", [{ name: " " }]],
  ])("skips what an incomplete Loop does not declare: %s", (_, list) => {
    const config = loop.data.loopConfig!;
    const usable = (entries: unknown, kept: unknown): unknown =>
      Array.isArray(entries) ? [...entries, kept] : entries;
    const incomplete = node(loop.id, {
      ...loop.data,
      loopConfig: {
        ...config,
        outputs: usable(list, config.outputs[0]),
        variables: usable(list, config.variables[0]),
      } as unknown as typeof config,
    });
    const own = (consumer: string | undefined): string[] =>
      deriveWorkflowVariableCatalog(
        nodes.map((candidate) =>
          candidate.id === loop.id ? incomplete : candidate,
        ),
        edges,
        consumer,
        [],
      )
        .filter((entry) => entry.sourceNodeId === loop.id)
        .map((entry) => entry.variableName);
    const kept = (name: string): string[] =>
      Array.isArray(list) ? [name] : [];
    expect(own("after")).toEqual(kept("result"));
    expect(own(undefined)).toEqual(kept("result"));
    expect(own(loopAgent.id)).toEqual(kept("value"));
  });

  it("offers body members the loop variables and the Loop's upstream", () => {
    const visible = selectors(loopAgent.id);
    expect(visible).toEqual(
      expect.arrayContaining([
        `${loop.id}.value`,
        "start.input",
        "start.topic",
      ]),
    );
    // The round Start assigns nothing; the Loop's exposed outputs are not yet produced.
    expect(visible).not.toContain(`${roundStart.id}.input`);
    expect(visible).not.toContain(`${loop.id}.result`);
    expect(visible).not.toContain(`${loop.id}.output`);
    const value = deriveWorkflowVariableCatalog(
      nodes,
      edges,
      loopAgent.id,
      [],
    ).find((entry) => entry.selector.join(".") === `${loop.id}.value`);
    expect(value?.valueType).toBe("string");
  });

  it("shares in-body products between body members only", () => {
    const reviewer = node(
      "reviewer",
      {
        kind: "agent",
        title: "Review",
        description: "",
        containerId: loop.id,
      },
      loop.id,
    );
    const visible = deriveWorkflowVariableCatalog(
      [...nodes, reviewer],
      [...edges, { id: "a-r", source: loopAgent.id, target: "reviewer" }],
      "reviewer",
      [],
    ).map((entry) => entry.selector.join("."));
    expect(visible).toContain(`${loopAgent.id}.output`);
    expect(visible).toContain(`${loop.id}.value`);
  });

  it("lists no round Start variables when the Loop panel derives body candidates", () => {
    const body = deriveWorkflowVariableCatalog(
      [roundStart, loopAgent],
      [],
      undefined,
      [],
    );
    expect(body.map((entry) => entry.selector.join("."))).toEqual([
      `${loopAgent.id}.output`,
    ]);
  });
});

describe("deriveWorkflowVariableCatalog loop body aggregators and membership", () => {
  const group = createMockWorkflowLoopGroup({
    sequence: 1,
    position: { x: 0, y: 0 },
    locale: "zh-CN",
  });
  const [loop, roundStart, loopAgent] = group.nodes as [
    Node<WorkflowNodeData, "workflow">,
    Node<WorkflowNodeData, "workflow">,
    Node<WorkflowNodeData, "workflow">,
  ];
  const start = node("start", {
    kind: "start",
    title: "Start",
    description: "",
  });
  const aggregator = node(
    "agg",
    {
      kind: "aggregator",
      title: "Agg",
      description: "",
      containerId: loop.id,
      aggregatorConfig: { variables: [] },
    },
    loop.id,
  );
  const edges: Edge[] = [
    { id: "s-l", source: "start", target: loop.id },
    ...group.edges,
    { id: "a-g", source: loopAgent.id, target: "agg" },
  ];

  it("offers a body aggregator only globals and in-body products, as publication types it", () => {
    const visible = deriveWorkflowVariableCatalog(
      [start, loop, roundStart, loopAgent, aggregator],
      edges,
      "agg",
      [{ name: "sys.workflow_id", valueType: "string" }],
    ).map((entry) => entry.selector.join("."));
    expect(visible).toEqual(["sys.workflow_id", `${loopAgent.id}.output`]);
  });

  it("treats a node as a Loop member by containerId, not by parentId alone", () => {
    // A parentId-only child of a Loop is refused by the backend; the catalog does not treat
    // it as a body member either, so it gets no loop variables.
    const stray = node(
      "stray",
      { kind: "agent", title: "Stray", description: "" },
      loop.id,
    );
    const visible = deriveWorkflowVariableCatalog(
      [start, loop, roundStart, loopAgent, stray],
      [...edges, { id: "a-s", source: loopAgent.id, target: "stray" }],
      "stray",
      [],
    ).map((entry) => entry.selector.join("."));
    expect(visible).not.toContain(`${loop.id}.value`);
  });
});
