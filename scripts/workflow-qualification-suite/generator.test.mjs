import assert from "node:assert/strict";
import test from "node:test";
import { generateAllWorkflows, layoutGraph } from "./generator.mjs";

/** Keeps executable payloads visible when comparing layout-only repairs. */
function executableContent(graph) {
  return {
    nodes: graph.nodes.map((node) => ({
      id: node.id,
      parentId: node.parentId,
      data: node.data,
    })),
    edges: graph.edges.map(
      ({ source, target, sourceHandle, targetHandle, data }) => ({
        source,
        target,
        sourceHandle,
        targetHandle,
        data,
      }),
    ),
    globalVariables: graph.globalVariables,
    description: graph.description,
    extra: graph.extra,
  };
}

test("layout repairs geometry and unique edge IDs without mutating execution data", () => {
  const graph = {
    description: "Keep the authored contract",
    extra: { future: true },
    globalVariables: [
      { name: "environment.key", valueType: "string", value: "kept" },
    ],
    nodes: [
      {
        id: "start",
        position: { x: 1, y: 2 },
        data: { kind: "start", input: "kept" },
      },
      {
        id: "agent",
        position: { x: 10, y: "invalid" },
        data: {
          kind: "agent",
          agentConfig: {
            prompt: "kept",
            roleId: "reviewer",
            skills: [{ skillId: "review", enabled: false }],
          },
        },
      },
      { id: "out", data: { kind: "output" } },
    ],
    edges: [
      { source: "start", target: "agent", data: { custom: 7 } },
      { id: "workflow-edge-1", source: "agent", target: "out" },
      { id: "workflow-edge-1", source: "start", target: "out" },
    ],
    viewport: { x: 0, y: 0, zoom: -1 },
  };
  const original = structuredClone(graph);
  const repaired = layoutGraph(graph);
  assert.deepEqual(graph, original);
  assert.deepEqual(executableContent(repaired), executableContent(original));
  assert.deepEqual(repaired.nodes[0].position, { x: 1, y: 2 });
  assert.deepEqual(repaired.viewport, { x: 0, y: 0, zoom: 1 });
  assert.equal(repaired.edges[1].id, "workflow-edge-1");
  assert.equal(new Set(repaired.edges.map((edge) => edge.id)).size, 3);
  for (const node of repaired.nodes) {
    assert.equal(node.type, "workflow");
    assert.ok(
      Number.isFinite(node.position.x) && Number.isFinite(node.position.y),
    );
  }
  assert.deepEqual(layoutGraph(repaired), repaired);
});

test("layout keeps Loop and Iteration ownership explicit, preserving missing Agent settings", () => {
  const graph = {
    nodes: [
      { id: "loop", data: { kind: "loop", loopConfig: { preserved: true } } },
      {
        id: "loop-start",
        parentId: "loop",
        data: { kind: "start", containerId: "loop" },
      },
      {
        id: "loop-agent",
        parentId: "loop",
        data: { kind: "agent", containerId: "loop" },
      },
      {
        id: "iter",
        data: { kind: "iteration", iterationConfig: { preserved: true } },
      },
      { id: "iter-agent", parentId: "iter", data: { kind: "agent" } },
    ],
    edges: [
      { source: "loop-start", target: "loop-agent" },
      { source: "iter", target: "iter-agent", sourceHandle: "iteration-entry" },
    ],
  };
  const repaired = layoutGraph(graph);
  assert.equal(repaired.schemaVersion, 2);
  assert.deepEqual(executableContent(repaired), executableContent(graph));
  assert.deepEqual(layoutGraph(repaired), repaired);
  assert.equal(Object.hasOwn(repaired.nodes[2].data, "agentConfig"), false);
  assert.equal(Object.hasOwn(repaired.nodes[4].data, "containerId"), false);
});

test("layout rejects malformed structure instead of changing execution ownership", () => {
  const start = { id: "start", data: { kind: "start" } };
  const iter = { id: "iter", data: { kind: "iteration" } };
  const cases = [
    { nodes: [null], edges: [] },
    { nodes: [start], edges: [null] },
    { nodes: [start, start], edges: [] },
    { nodes: [start], edges: [{ source: "start", target: "missing" }] },
    { nodes: [{ ...start, parentId: "missing" }], edges: [] },
    {
      nodes: [
        iter,
        {
          id: "child",
          parentId: "iter",
          data: { kind: "agent", containerId: "iter" },
        },
      ],
      edges: [],
    },
    {
      nodes: [
        { id: "loop", data: { kind: "loop" } },
        { id: "child", parentId: "loop", data: { kind: "agent" } },
      ],
      edges: [],
    },
    {
      nodes: [
        iter,
        start,
        { id: "child", parentId: "iter", data: { kind: "agent" } },
      ],
      edges: [{ source: "start", target: "child" }],
    },
  ];
  for (const graph of cases) {
    const before = structuredClone(graph);
    assert.throws(
      () => layoutGraph(graph),
      /node|edge|ownership|container|parent/i,
    );
    assert.deepEqual(graph, before);
  }
});

test("all 112 generated graphs have complete Agent contracts and legal scope boundaries", () => {
  const workflows = generateAllWorkflows();
  assert.equal(workflows.length, 112);
  assert.deepEqual(generateAllWorkflows(), workflows);
  for (const workflow of workflows) {
    const { graph } = workflow;
    assert.equal(graph.schemaVersion, 2);
    assert.deepEqual(layoutGraph(graph), graph);
    const byId = new Map(graph.nodes.map((node) => [node.id, node]));
    assert.equal(byId.size, graph.nodes.length);
    assert.equal(
      new Set(graph.edges.map((edge) => edge.id)).size,
      graph.edges.length,
    );
    for (const node of graph.nodes) {
      assert.ok(
        Number.isFinite(node.position.x) && Number.isFinite(node.position.y),
      );
      if (node.parentId) {
        const owner = byId.get(node.parentId);
        assert.ok(owner, `Scenario ${workflow.index}: missing owner`);
        assert.equal(
          node.data.containerId,
          owner.data.kind === "loop" ? owner.id : undefined,
        );
      }
      if (node.data.kind === "agent") {
        const config = node.data.agentConfig;
        if (workflow.isSuperLong) {
          // Super-long agents extend the base contract with frozen execution policies.
          assert.equal(config.schemaVersion, 3);
          assert.deepEqual(config.executor, {
            agentCli: "official/ora-space.opencode",
            modelId: "bluezone/zhipu/glm-5.3",
          });
          assert.equal(config.roleId, "");
          assert.deepEqual(config.skills, []);
          assert.deepEqual(config.mcps, []);
          assert.equal(config.interactive, false);
          assert.ok(["wait", "timeout"].includes(config.promptInactivity));
          if (config.retry) {
            assert.deepEqual(Object.keys(config.retry).sort(), [
              "enabled",
              "initialDelaySeconds",
              "maxRetries",
            ]);
          }
          if (config.outputContract) {
            assert.equal(config.outputContract.type, "structured");
          }
        } else {
          assert.deepEqual(config, {
            schemaVersion: 3,
            executor: {
              agentCli: "official/ora-space.opencode",
              modelId: "bluezone/zhipu/glm-5.3",
            },
            roleId: "",
            skills: [],
            mcps: [],
            prompt: config.prompt,
            interactive: false,
          });
        }
        assert.equal(typeof config.prompt, "string");
      }
    }
    for (const edge of graph.edges) {
      const source = byId.get(edge.source);
      const target = byId.get(edge.target);
      assert.ok(source && target);
      if (source.parentId !== target.parentId) {
        assert.deepEqual(
          [source.data.kind, target.parentId, edge.sourceHandle],
          ["iteration", source.id, "iteration-entry"],
        );
      }
    }
  }
});

test("the super-long batch keeps the endurance class bars and required kinds", () => {
  const workflows = generateAllWorkflows().filter(
    (workflow) => workflow.isSuperLong,
  );
  assert.equal(workflows.length, 11);
  for (const workflow of workflows) {
    const kinds = new Set(workflow.graph.nodes.map((node) => node.data.kind));
    for (const kind of ["condition", "aggregator", "iteration", "loop"]) {
      assert.ok(kinds.has(kind), `${workflow.name} lacks ${kind}`);
    }
    assert.ok(
      workflow.graph.nodes.length >= 15,
      `${workflow.name} has fewer than 15 nodes`,
    );
    assert.ok(
      workflow.deepNodeCount >= 10,
      `${workflow.name} has fewer than 10 deep nodes`,
    );
    const deepPolicies = workflow.graph.nodes
      .filter((node) => node.data.kind === "agent")
      .map((node) => node.data.agentConfig.promptInactivity);
    assert.ok(deepPolicies.every((policy) => policy !== undefined));
  }
});

test("Loop matrix prompts match the selected termination token and carried state", () => {
  const workflows = generateAllWorkflows();
  for (const index of [41, 42, 43, 44, 45, 46, 47, 48]) {
    const workflow = workflows.find((entry) => entry.index === index);
    const loop = workflow.graph.nodes.find((node) => node.id === "loop");
    const agent = workflow.graph.nodes.find((node) => node.id === "loop-agent");
    const condition = loop.data.loopConfig.until.conditions[0];
    if (condition.value !== "") {
      assert.ok(agent.data.agentConfig.prompt.includes(condition.value));
    }
    if (loop.data.loopConfig.maxIterations > 1) {
      assert.ok(
        agent.data.agentConfig.prompt.includes("{{#loop.loop_state#}}"),
      );
    }
  }
});

test("non-finite geometry and authoring cycles produce finite, stable display coordinates", () => {
  const graph = {
    nodes: [
      {
        id: "a",
        position: { x: Number.NaN, y: 0 },
        initialWidth: "bad",
        data: { kind: "agent" },
      },
      {
        id: "b",
        position: { x: 10, y: Number.POSITIVE_INFINITY },
        data: { kind: "agent" },
      },
    ],
    edges: [
      { source: "a", target: "b" },
      { source: "b", target: "a" },
    ],
  };
  const repaired = layoutGraph(graph);
  assert.deepEqual(executableContent(repaired), executableContent(graph));
  assert.ok(
    repaired.nodes.every(
      (node) =>
        Number.isFinite(node.position.x) && Number.isFinite(node.position.y),
    ),
  );
  assert.equal(Object.hasOwn(repaired.nodes[0], "initialWidth"), false);
  assert.deepEqual(layoutGraph(repaired), repaired);
});
