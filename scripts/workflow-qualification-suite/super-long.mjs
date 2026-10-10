// scripts/workflow-qualification-suite/super-long.mjs
// Generates the super-long endurance workflows (W102-W112) for batch long-run qualification.
// Every super-long workflow:
// 1. Contains at least 15 nodes and at least 10 deep (long-running) agent nodes.
// 2. Still embeds all four required control kinds: Condition, Aggregator, Iteration, Loop.
// 3. Freezes `agentConfig.promptInactivity` (mostly "wait") so long silent turns are tolerated.
// 4. Varies a structural axis per workflow: serial depth, parallel width, iteration load,
//    loop rounds, retry policies, structured output, routing, and configuration boundaries.

import { agentConfiguration } from "./generator.mjs";

/** Topics rotated across deep stages so long chains never repeat the same instruction. */
const TOPICS = [
  "runtime architecture and IPC boundaries",
  "SQLite concurrency, WAL, and crash recovery",
  "process supervision, reaping, and orphan detection",
  "WebView sandboxing and origin isolation",
  "workflow DAG scheduling and join semantics",
  "variable-pool typing and template rendering",
  "retry, backoff, and failure classification",
  "session lifecycle, handoff, and transcript persistence",
  "plugin packaging, installation, and integrity",
  "observability, logging, and request lifecycle tracing",
  "resource leaks, memory ceilings, and steady-state behavior",
  "release readiness and regression risk",
];

/** The shared instruction that makes a turn long: one read-only listing plus a written report. */
const DEEP_MARKER = "read-only directory listing";

/** Builds a deep agent node whose single turn is expected to run for minutes. */
function deepAgent(
  id,
  title,
  stage,
  { policy = "wait", retry, structured } = {},
) {
  const config = agentConfiguration(
    [
      `Long-running endurance stage ${stage}: ${title}.`,
      `1) Run exactly one ${DEEP_MARKER} of your current working directory; never create, modify, or delete any file.`,
      `2) Then write a detailed technical report on ${
        TOPICS[stage % TOPICS.length]
      }.`,
      "Include an executive summary, at least 5 numbered sections, a short risk list, and a conclusion.",
      "Aim for roughly 700-1000 words. Do not ask questions.",
      `End your reply with the exact line: STAGE ${stage} COMPLETE`,
    ].join(" "),
  );
  config.promptInactivity = policy;
  if (retry) config.retry = retry;
  if (structured) {
    config.outputContract = { type: "structured", schema: structured };
  }
  return { id, data: { kind: "agent", title, agentConfig: config } };
}

/** Deep prompt for one iteration round; the collected output is the round report. */
function iterationPrompt(prefix) {
  return [
    `Iteration item: {{#${prefix}.item#}} (index {{#${prefix}.index#}}).`,
    `1) Run exactly one ${DEEP_MARKER}; never modify anything.`,
    "2) Write a ~400-word round report applying your assigned topic to this item, with 4 numbered sections.",
    "End with the exact line: ITEM_COMPLETE",
  ].join(" ");
}

/** Deep prompt for one loop round with an explicit state machine and terminal token. */
function loopPrompt(prefix, variable, states, terminal) {
  const table = states
    .slice(0, -1)
    .map(
      (state, round) =>
        `state ${JSON.stringify(state)} => end with token ${JSON.stringify(
          states[round + 1],
        )}`,
    )
    .join("; ");
  return [
    `Current carried state: {{#${prefix}.${variable}#}}.`,
    `1) Run exactly one ${DEEP_MARKER}; never modify anything.`,
    "2) Write a ~350-word round report on your assigned topic with 3 numbered sections.",
    `3) State machine (apply the first matching rule): ${table}.`,
    "Reply with your report and end with exactly the token for the current state.",
    `The uppercase string ${JSON.stringify(
      terminal,
    )} may appear only as the final token of the final round.`,
  ].join(" ");
}

/** Counts definition nodes this generator marks as deep (long-running) agents. */
function countDeepAgents(graph) {
  return graph.nodes.filter(
    (node) =>
      node.data.kind === "agent" &&
      typeof node.data.agentConfig?.prompt === "string" &&
      node.data.agentConfig.prompt.includes(DEEP_MARKER),
  ).length;
}

/** Wraps a finished graph with its workflow metadata for the runner. */
function finalize(index, name, category, description, graph) {
  return {
    index,
    name,
    category,
    description,
    isSuperLong: true,
    deepNodeCount: countDeepAgents(graph),
    graph,
  };
}

/** Shared prologue nodes: start, condition gate, deep branches, first aggregator. */
function prologueNodes(index, items, { branchA = 1, branchB = 2 } = {}) {
  return [
    {
      id: "start",
      data: {
        kind: "start",
        title: `Super-Long ${index} Entry`,
        inputVariables: [
          { name: "route", valueType: "string", value: "a" },
          { name: "items", valueType: "array[string]", value: items },
          {
            name: "items2",
            valueType: "array[string]",
            value: ["x-ray", "yankee", "zulu"],
          },
          { name: "meta", valueType: "string", value: `superlong-${index}` },
        ],
      },
    },
    {
      id: "cond",
      data: {
        kind: "condition",
        title: "Route Gate",
        cases: [
          {
            id: "case-a",
            logic: "and",
            conditions: [
              {
                variableSelector: ["start", "route"],
                operator: "equals",
                value: "a",
              },
            ],
          },
        ],
      },
    },
    deepAgent("agent-a", "Branch A Deep Processor", branchA),
    deepAgent("agent-b", "Branch B Deep Processor (ELSE)", branchB),
    {
      id: "agg",
      data: {
        kind: "aggregator",
        title: "Branch Aggregator",
        aggregatorConfig: {
          variables: [
            ["agent-a", "output"],
            ["agent-b", "output"],
          ],
        },
      },
    },
  ];
}

/** One iteration container with a single deep item worker. */
function iterationNodes(
  prefix,
  { sourceVar = "items", maxIterations = 50, strategy = "continue" } = {},
) {
  return [
    {
      id: prefix,
      initialWidth: 640,
      initialHeight: 340,
      data: {
        kind: "iteration",
        title: `Deep Iteration ${prefix}`,
        iterationConfig: {
          iteratorSelector: ["start", sourceVar],
          collectSelector: [`${prefix}-agent`, "output"],
          errorStrategy: strategy,
          maxIterations,
        },
      },
    },
    {
      id: `${prefix}-agent`,
      parentId: prefix,
      data: {
        kind: "agent",
        title: `Iteration Deep Worker ${prefix}`,
        agentConfig: (() => {
          const config = agentConfiguration(iterationPrompt(prefix));
          config.promptInactivity = "wait";
          return config;
        })(),
      },
    },
  ];
}

/** One loop container with a deep round worker and a deterministic state machine. */
function loopNodes(
  prefix,
  { rounds = 3, maxIterations = 6, variable = "loop_state" } = {},
) {
  const states = [
    "init",
    ...Array.from({ length: rounds - 1 }, (_, round) => `ROUND_${round + 1}`),
    "STATE_DONE",
  ];
  return [
    {
      id: prefix,
      initialWidth: 620,
      initialHeight: 300,
      data: {
        kind: "loop",
        title: `Deep Loop ${prefix}`,
        loopConfig: {
          maxIterations,
          variables: [
            {
              name: variable,
              valueType: "string",
              initial: { kind: "constant", value: "init" },
              feedback: [`${prefix}-agent`, "output"],
            },
          ],
          until: {
            logic: "and",
            conditions: [
              {
                variableSelector: [`${prefix}-agent`, "output"],
                operator: "contains",
                value: "STATE_DONE",
              },
            ],
          },
          outputs: [
            {
              name: "result",
              variableSelector: [`${prefix}-agent`, "output"],
            },
          ],
        },
      },
    },
    {
      id: `${prefix}-start`,
      parentId: prefix,
      data: { kind: "start", containerId: prefix },
    },
    {
      id: `${prefix}-agent`,
      parentId: prefix,
      data: {
        kind: "agent",
        containerId: prefix,
        title: `Loop Deep Worker ${prefix}`,
        agentConfig: (() => {
          const config = agentConfiguration(
            loopPrompt(prefix, variable, states, "STATE_DONE"),
          );
          config.promptInactivity = "wait";
          return config;
        })(),
      },
    },
  ];
}

/** Terminal output node. */
function outputNode(outputs) {
  return {
    id: "out",
    data: { kind: "output", title: "Super-Long Result Output", outputs },
  };
}

/** Standard start/condition/branch/aggregator edges. */
const prologueEdges = [
  { source: "start", target: "cond" },
  { source: "cond", sourceHandle: "case-a", target: "agent-a" },
  { source: "cond", sourceHandle: "else", target: "agent-b" },
  { source: "agent-a", target: "agg" },
  { source: "agent-b", target: "agg" },
];

// ---------------------------------------------------------------------------
// W102: serial deep chain — 8-stage pipeline between the aggregator and iteration.
// ---------------------------------------------------------------------------
function w102() {
  const nodes = [
    ...prologueNodes(102, ["alpha", "beta", "gamma", "delta"]),
    deepAgent("d1", "Serial Stage 1", 5),
    deepAgent("d2", "Serial Stage 2", 6),
    deepAgent("d3", "Serial Stage 3", 7),
    deepAgent("d4", "Serial Stage 4", 8),
    deepAgent("d5", "Serial Stage 5", 9),
    deepAgent("d6", "Serial Stage 6", 10),
    deepAgent("d7", "Serial Stage 7", 11),
    deepAgent("d8", "Serial Stage 8", 12),
    ...iterationNodes("iter"),
    deepAgent("d9", "Post-Iteration Synthesis", 1),
    ...loopNodes("loop", { rounds: 3, maxIterations: 5 }),
    deepAgent("d10", "Final Sign-off", 4),
    outputNode([
      { name: "branch", variableSelector: ["agg", "output"] },
      { name: "iterated", variableSelector: ["iter", "output"] },
      { name: "loop_result", variableSelector: ["loop", "result"] },
      { name: "final", variableSelector: ["d10", "output"] },
    ]),
  ];
  const edges = [
    ...prologueEdges,
    { source: "agg", target: "d1" },
    { source: "d1", target: "d2" },
    { source: "d2", target: "d3" },
    { source: "d3", target: "d4" },
    { source: "d4", target: "d5" },
    { source: "d5", target: "d6" },
    { source: "d6", target: "d7" },
    { source: "d7", target: "d8" },
    { source: "d8", target: "iter" },
    { source: "iter", sourceHandle: "iteration-entry", target: "iter-agent" },
    { source: "iter", target: "d9" },
    { source: "d9", target: "loop" },
    { source: "loop-start", target: "loop-agent" },
    { source: "loop", target: "d10" },
    { source: "d10", target: "out" },
  ];
  return finalize(
    102,
    "wf-102-superlong-serial-chain",
    "Super-Long: Serial Deep Chain (8 stages)",
    "Serial 8-stage deep pipeline with a 4-item iteration and a 3-round loop; all deep nodes wait on inactivity.",
    { nodes, edges },
  );
}

// ---------------------------------------------------------------------------
// W103: wide parallel fan — 6 concurrent deep agents joined by a deep synthesizer.
// ---------------------------------------------------------------------------
function w103() {
  const nodes = [
    ...prologueNodes(103, ["alpha", "beta", "gamma", "delta"], {
      branchA: 3,
      branchB: 4,
    }),
    deepAgent("p1", "Parallel Probe 1", 1),
    deepAgent("p2", "Parallel Probe 2", 2),
    deepAgent("p3", "Parallel Probe 3", 3),
    deepAgent("p4", "Parallel Probe 4", 4),
    deepAgent("p5", "Parallel Probe 5", 5),
    deepAgent("p6", "Parallel Probe 6", 6),
    deepAgent("join", "Parallel Join Synthesizer", 7),
    ...iterationNodes("iter"),
    ...loopNodes("loop", { rounds: 3, maxIterations: 5 }),
    deepAgent("d9", "Post-Loop Verifier", 10),
    outputNode([
      { name: "branch", variableSelector: ["agg", "output"] },
      { name: "join", variableSelector: ["join", "output"] },
      { name: "iterated", variableSelector: ["iter", "output"] },
      { name: "loop_result", variableSelector: ["loop", "result"] },
    ]),
  ];
  const edges = [
    ...prologueEdges,
    { source: "agg", target: "p1" },
    { source: "agg", target: "p2" },
    { source: "agg", target: "p3" },
    { source: "agg", target: "p4" },
    { source: "agg", target: "p5" },
    { source: "agg", target: "p6" },
    { source: "p1", target: "join" },
    { source: "p2", target: "join" },
    { source: "p3", target: "join" },
    { source: "p4", target: "join" },
    { source: "p5", target: "join" },
    { source: "p6", target: "join" },
    { source: "join", target: "iter" },
    { source: "iter", sourceHandle: "iteration-entry", target: "iter-agent" },
    { source: "iter", target: "loop" },
    { source: "loop-start", target: "loop-agent" },
    { source: "loop", target: "d9" },
    { source: "d9", target: "out" },
  ];
  return finalize(
    103,
    "wf-103-superlong-parallel-fan",
    "Super-Long: Wide Parallel Fan (6-way)",
    "Six concurrent deep agents joined by a synthesizer, then iteration and loop; exercises parallel long sessions.",
    { nodes, edges },
  );
}

// ---------------------------------------------------------------------------
// W104: iteration-heavy — two iteration containers (continue + fail strategies).
// ---------------------------------------------------------------------------
function w104() {
  const nodes = [
    ...prologueNodes(
      104,
      ["item-1", "item-2", "item-3", "item-4", "item-5", "item-6"],
      {
        branchA: 5,
        branchB: 6,
      },
    ),
    ...iterationNodes("iter", { strategy: "continue" }),
    deepAgent("d1", "Inter-Iteration Bridge", 8),
    deepAgent("d2", "Inter-Iteration Audit", 9),
    ...iterationNodes("iter2", {
      sourceVar: "items2",
      strategy: "fail",
      maxIterations: 10,
    }),
    deepAgent("d3", "Post-Iteration Consolidation", 10),
    ...loopNodes("loop", { rounds: 3, maxIterations: 5 }),
    deepAgent("d4", "Final Verdict", 12),
    deepAgent("d5", "Endurance Cross-Check", 3),
    outputNode([
      { name: "branch", variableSelector: ["agg", "output"] },
      { name: "iterated", variableSelector: ["iter", "output"] },
      { name: "iterated2", variableSelector: ["iter2", "output"] },
      { name: "loop_result", variableSelector: ["loop", "result"] },
    ]),
  ];
  const edges = [
    ...prologueEdges,
    { source: "agg", target: "iter" },
    { source: "iter", sourceHandle: "iteration-entry", target: "iter-agent" },
    { source: "iter", target: "d1" },
    { source: "d1", target: "d2" },
    { source: "d2", target: "iter2" },
    { source: "iter2", sourceHandle: "iteration-entry", target: "iter2-agent" },
    { source: "iter2", target: "d3" },
    { source: "d3", target: "loop" },
    { source: "loop-start", target: "loop-agent" },
    { source: "loop", target: "d4" },
    { source: "d4", target: "d5" },
    { source: "d5", target: "out" },
  ];
  return finalize(
    104,
    "wf-104-superlong-iteration-heavy",
    "Super-Long: Iteration Heavy (6+3 items, continue+fail)",
    "Two iteration containers with contrasting error strategies over deep item workers, plus loop and chain.",
    { nodes, edges },
  );
}

// ---------------------------------------------------------------------------
// W105: loop-heavy — two loops (6 rounds and 3 rounds) around a deep chain.
// ---------------------------------------------------------------------------
function w105() {
  const nodes = [
    ...prologueNodes(105, ["alpha", "beta", "gamma"], {
      branchA: 7,
      branchB: 8,
    }),
    ...loopNodes("loop1", {
      rounds: 6,
      maxIterations: 8,
      variable: "state_one",
    }),
    deepAgent("d1", "Inter-Loop Synthesis", 10),
    ...loopNodes("loop2", {
      rounds: 3,
      maxIterations: 5,
      variable: "state_two",
    }),
    ...iterationNodes("iter"),
    deepAgent("d2", "Post-Loop Aggregation", 1),
    deepAgent("d3", "Steady-State Review", 2),
    deepAgent("d4", "Endurance Report", 3),
    deepAgent("d5", "Loop-Heavy Cross-Check", 4),
    outputNode([
      { name: "branch", variableSelector: ["agg", "output"] },
      { name: "loop1_result", variableSelector: ["loop1", "result"] },
      { name: "loop2_result", variableSelector: ["loop2", "result"] },
      { name: "iterated", variableSelector: ["iter", "output"] },
      { name: "final", variableSelector: ["d5", "output"] },
    ]),
  ];
  const edges = [
    ...prologueEdges,
    { source: "agg", target: "loop1" },
    { source: "loop1-start", target: "loop1-agent" },
    { source: "loop1", target: "d1" },
    { source: "d1", target: "loop2" },
    { source: "loop2-start", target: "loop2-agent" },
    { source: "loop2", target: "iter" },
    { source: "iter", sourceHandle: "iteration-entry", target: "iter-agent" },
    { source: "iter", target: "d2" },
    { source: "d2", target: "d3" },
    { source: "d3", target: "d4" },
    { source: "d4", target: "d5" },
    { source: "d5", target: "out" },
  ];
  return finalize(
    105,
    "wf-105-superlong-loop-heavy",
    "Super-Long: Loop Heavy (6+3 rounds)",
    "Two sequential loops (6 and 3 rounds) with deep round workers, then a 3-item iteration and closing chain.",
    { nodes, edges },
  );
}

// ---------------------------------------------------------------------------
// W106: wait-policy matrix — every agent waits; extra-long single turns.
// ---------------------------------------------------------------------------
function w106() {
  const deepWith = (id, title, stage) => {
    const config = agentConfiguration(
      [
        `Long-running endurance stage ${stage}: extra-long document ${stage}.`,
        `1) Run exactly one ${DEEP_MARKER} of your current working directory; never modify anything.`,
        `2) Write an exhaustive technical dossier on ${
          TOPICS[stage % TOPICS.length]
        }.`,
        "Structure: executive summary, 8 numbered sections, a comparison table, a risk register, recommendations, and a conclusion.",
        "Aim for roughly 1100-1500 words. Do not ask questions.",
        `End your reply with the exact line: STAGE ${stage} COMPLETE`,
      ].join(" "),
    );
    config.promptInactivity = "wait";
    return { id, data: { kind: "agent", title, agentConfig: config } };
  };
  const nodes = [
    ...prologueNodes(106, ["alpha", "beta", "gamma", "delta"], {
      branchA: 9,
      branchB: 10,
    }),
    deepWith("w1", "Dossier Stage 1", 1),
    deepWith("w2", "Dossier Stage 2", 2),
    deepWith("w3", "Dossier Stage 3", 3),
    deepWith("w4", "Dossier Stage 4", 4),
    deepWith("w5", "Dossier Stage 5", 5),
    deepWith("w6", "Dossier Stage 6", 6),
    ...iterationNodes("iter"),
    ...loopNodes("loop", { rounds: 3, maxIterations: 5 }),
    deepWith("w7", "Dossier Stage 7", 7),
    deepWith("w8", "Dossier Stage 8", 8),
    outputNode([
      { name: "branch", variableSelector: ["agg", "output"] },
      { name: "iterated", variableSelector: ["iter", "output"] },
      { name: "loop_result", variableSelector: ["loop", "result"] },
      { name: "final", variableSelector: ["w8", "output"] },
    ]),
  ];
  const edges = [
    ...prologueEdges,
    { source: "agg", target: "w1" },
    { source: "w1", target: "w2" },
    { source: "w2", target: "w3" },
    { source: "w3", target: "w4" },
    { source: "w4", target: "w5" },
    { source: "w5", target: "w6" },
    { source: "w6", target: "iter" },
    { source: "iter", sourceHandle: "iteration-entry", target: "iter-agent" },
    { source: "iter", target: "loop" },
    { source: "loop-start", target: "loop-agent" },
    { source: "loop", target: "w7" },
    { source: "w7", target: "w8" },
    { source: "w8", target: "out" },
  ];
  return finalize(
    106,
    "wf-106-superlong-wait-matrix",
    "Super-Long: Prompt-Inactivity Wait Matrix",
    "Every agent freezes promptInactivity=wait and writes extra-long documents; probes silence tolerance end to end.",
    { nodes, edges },
  );
}

// ---------------------------------------------------------------------------
// W107: retry-policy matrix — deep agents with varied retry configurations.
// ---------------------------------------------------------------------------
function w107() {
  const withRetry = (id, title, stage, policy, retry) =>
    deepAgent(id, title, stage, { policy, retry });
  const nodes = [
    ...prologueNodes(107, ["alpha", "beta", "gamma", "delta"], {
      branchA: 11,
      branchB: 12,
    }),
    withRetry("r1", "Retry Stage 1 (off)", 1, "timeout", {
      enabled: false,
      maxRetries: 0,
      initialDelaySeconds: 5,
    }),
    withRetry("r2", "Retry Stage 2 (max 1)", 2, "wait", {
      enabled: true,
      maxRetries: 1,
      initialDelaySeconds: 5,
    }),
    withRetry("r3", "Retry Stage 3 (max 2)", 3, "timeout", {
      enabled: true,
      maxRetries: 2,
      initialDelaySeconds: 5,
    }),
    withRetry("r4", "Retry Stage 4 (max 3, wait)", 4, "wait", {
      enabled: true,
      maxRetries: 3,
      initialDelaySeconds: 5,
    }),
    withRetry("r5", "Retry Stage 5 (default)", 5, "wait"),
    withRetry("r6", "Retry Stage 6 (default, timeout)", 6, "timeout"),
    ...iterationNodes("iter"),
    ...loopNodes("loop", { rounds: 3, maxIterations: 5 }),
    withRetry("r7", "Retry Stage 7 (max 2, wait)", 9, "wait", {
      enabled: true,
      maxRetries: 2,
      initialDelaySeconds: 10,
    }),
    outputNode([
      { name: "branch", variableSelector: ["agg", "output"] },
      { name: "iterated", variableSelector: ["iter", "output"] },
      { name: "loop_result", variableSelector: ["loop", "result"] },
      { name: "final", variableSelector: ["r7", "output"] },
    ]),
  ];
  const edges = [
    ...prologueEdges,
    { source: "agg", target: "r1" },
    { source: "r1", target: "r2" },
    { source: "r2", target: "r3" },
    { source: "r3", target: "r4" },
    { source: "r4", target: "r5" },
    { source: "r5", target: "r6" },
    { source: "r6", target: "iter" },
    { source: "iter", sourceHandle: "iteration-entry", target: "iter-agent" },
    { source: "iter", target: "loop" },
    { source: "loop-start", target: "loop-agent" },
    { source: "loop", target: "r7" },
    { source: "r7", target: "out" },
  ];
  return finalize(
    107,
    "wf-107-superlong-retry-matrix",
    "Super-Long: Retry Policy Matrix",
    "Deep agents with retry off / max 1 / 2 / 3 / default and mixed inactivity policies on long turns.",
    { nodes, edges },
  );
}

// ---------------------------------------------------------------------------
// W108: condition-router — two condition gates routing between deep branches.
// ---------------------------------------------------------------------------
function w108() {
  const nodes = [
    ...prologueNodes(108, ["alpha", "beta", "gamma", "delta"], {
      branchA: 1,
      branchB: 2,
    }),
    {
      id: "cond2",
      data: {
        kind: "condition",
        title: "Second Route Gate",
        cases: [
          {
            id: "case-c",
            logic: "and",
            conditions: [
              {
                variableSelector: ["start", "route"],
                operator: "equals",
                value: "a",
              },
            ],
          },
        ],
      },
    },
    deepAgent("agent-c", "Branch C Deep Processor", 3),
    deepAgent("agent-d", "Branch D Deep Processor (ELSE)", 4),
    {
      id: "agg2",
      data: {
        kind: "aggregator",
        title: "Second Branch Aggregator",
        aggregatorConfig: {
          variables: [
            ["agent-c", "output"],
            ["agent-d", "output"],
          ],
        },
      },
    },
    deepAgent("d1", "Router Stage 1", 5),
    deepAgent("d2", "Router Stage 2", 6),
    ...iterationNodes("iter"),
    deepAgent("d3", "Post-Iteration Router", 8),
    ...loopNodes("loop", { rounds: 3, maxIterations: 5 }),
    deepAgent("d4", "Final Router Verdict", 10),
    outputNode([
      { name: "branch1", variableSelector: ["agg", "output"] },
      { name: "branch2", variableSelector: ["agg2", "output"] },
      { name: "iterated", variableSelector: ["iter", "output"] },
      { name: "loop_result", variableSelector: ["loop", "result"] },
    ]),
  ];
  const edges = [
    ...prologueEdges,
    { source: "agg", target: "cond2" },
    { source: "cond2", sourceHandle: "case-c", target: "agent-c" },
    { source: "cond2", sourceHandle: "else", target: "agent-d" },
    { source: "agent-c", target: "agg2" },
    { source: "agent-d", target: "agg2" },
    { source: "agg2", target: "d1" },
    { source: "d1", target: "d2" },
    { source: "d2", target: "iter" },
    { source: "iter", sourceHandle: "iteration-entry", target: "iter-agent" },
    { source: "iter", target: "d3" },
    { source: "d3", target: "loop" },
    { source: "loop-start", target: "loop-agent" },
    { source: "loop", target: "d4" },
    { source: "d4", target: "out" },
  ];
  return finalize(
    108,
    "wf-108-superlong-condition-router",
    "Super-Long: Condition Router (2 gates, 4 branches)",
    "Two condition gates routing between four deep branches with two aggregators on a long chain.",
    { nodes, edges },
  );
}

// ---------------------------------------------------------------------------
// W109: structured-output chain — deep agents returning JSON contracts.
// ---------------------------------------------------------------------------
const STRUCTURED_SCHEMA = {
  type: "object",
  properties: {
    summary: { type: "string" },
    confidence: { type: "number" },
  },
  required: ["summary", "confidence"],
  additionalProperties: false,
};

function w109() {
  const structuredDeep = (id, title, stage) => {
    const node = deepAgent(id, title, stage, { structured: STRUCTURED_SCHEMA });
    node.data.agentConfig.prompt = [
      `Long-running endurance stage ${stage}: structured ${title}.`,
      `1) Run exactly one ${DEEP_MARKER} of your current working directory; never modify anything.`,
      `2) Analyze ${TOPICS[stage % TOPICS.length]} and produce your findings.`,
      "3) Reply with ONLY one bare JSON object of shape",
      '{"summary": "<a 150-250 word technical summary>", "confidence": <number 0.0-1.0>}',
      "No markdown, no prose outside the JSON object.",
    ].join(" ");
    return node;
  };
  const nodes = [
    ...prologueNodes(109, ["alpha", "beta", "gamma", "delta"], {
      branchA: 2,
      branchB: 3,
    }),
    structuredDeep("s1", "Structured Stage 1", 4),
    structuredDeep("s2", "Structured Stage 2", 5),
    structuredDeep("s3", "Structured Stage 3", 6),
    deepAgent("d1", "Plain Stage After Structured", 7),
    deepAgent("d2", "Second Plain Stage", 8),
    ...iterationNodes("iter"),
    ...loopNodes("loop", { rounds: 3, maxIterations: 5 }),
    structuredDeep("d3", "Final Structured Verdict", 11),
    outputNode([
      { name: "branch", variableSelector: ["agg", "output"] },
      { name: "structured1", variableSelector: ["s1", "structured_output"] },
      { name: "structured2", variableSelector: ["s2", "structured_output"] },
      { name: "structured3", variableSelector: ["s3", "structured_output"] },
      { name: "iterated", variableSelector: ["iter", "output"] },
      { name: "loop_result", variableSelector: ["loop", "result"] },
    ]),
  ];
  const edges = [
    ...prologueEdges,
    { source: "agg", target: "s1" },
    { source: "s1", target: "s2" },
    { source: "s2", target: "s3" },
    { source: "s3", target: "d1" },
    { source: "d1", target: "d2" },
    { source: "d2", target: "iter" },
    { source: "iter", sourceHandle: "iteration-entry", target: "iter-agent" },
    { source: "iter", target: "loop" },
    { source: "loop-start", target: "loop-agent" },
    { source: "loop", target: "d3" },
    { source: "d3", target: "out" },
  ];
  return finalize(
    109,
    "wf-109-superlong-structured-chain",
    "Super-Long: Structured Output Chain",
    "Deep agents constrained to bare-JSON structured output on long turns, mixed with plain deep stages.",
    { nodes, edges },
  );
}

// ---------------------------------------------------------------------------
// W110: aggregator-priority — two aggregators with different candidate orders.
// ---------------------------------------------------------------------------
function w110() {
  const nodes = [
    ...prologueNodes(110, ["alpha", "beta", "gamma", "delta"], {
      branchA: 4,
      branchB: 5,
    }),
    {
      id: "cond2",
      data: {
        kind: "condition",
        title: "Reverse Route Gate",
        cases: [
          {
            id: "case-c",
            logic: "and",
            conditions: [
              {
                variableSelector: ["start", "route"],
                operator: "not_equals",
                value: "zzz",
              },
            ],
          },
        ],
      },
    },
    deepAgent("agent-c", "Branch C Deep Processor", 6),
    deepAgent("agent-d", "Branch D Deep Processor (ELSE)", 7),
    {
      id: "agg2",
      data: {
        kind: "aggregator",
        title: "Reverse Priority Aggregator (d before c)",
        aggregatorConfig: {
          variables: [
            ["agent-d", "output"],
            ["agent-c", "output"],
          ],
        },
      },
    },
    deepAgent("d1", "Priority Stage 1", 8),
    deepAgent("d2", "Priority Stage 2", 9),
    ...iterationNodes("iter"),
    ...loopNodes("loop", { rounds: 3, maxIterations: 5 }),
    deepAgent("d3", "Priority Final", 12),
    deepAgent("d4", "Priority Cross-Check", 5),
    outputNode([
      { name: "priority1", variableSelector: ["agg", "output"] },
      { name: "priority2", variableSelector: ["agg2", "output"] },
      { name: "iterated", variableSelector: ["iter", "output"] },
      { name: "loop_result", variableSelector: ["loop", "result"] },
    ]),
  ];
  const edges = [
    ...prologueEdges,
    { source: "agg", target: "cond2" },
    { source: "cond2", sourceHandle: "case-c", target: "agent-c" },
    { source: "cond2", sourceHandle: "else", target: "agent-d" },
    { source: "agent-c", target: "agg2" },
    { source: "agent-d", target: "agg2" },
    { source: "agg2", target: "d1" },
    { source: "d1", target: "d2" },
    { source: "d2", target: "iter" },
    { source: "iter", sourceHandle: "iteration-entry", target: "iter-agent" },
    { source: "iter", target: "loop" },
    { source: "loop-start", target: "loop-agent" },
    { source: "loop", target: "d3" },
    { source: "d3", target: "d4" },
    { source: "d4", target: "out" },
  ];
  return finalize(
    110,
    "wf-110-superlong-aggregator-priority",
    "Super-Long: Aggregator Priority Orders",
    "Two aggregators with opposite candidate priority over routed deep branches on a long chain.",
    { nodes, edges },
  );
}

// ---------------------------------------------------------------------------
// W111: configuration boundaries — iteration source length equals maxIterations,
// loop maxIterations at the config ceiling (100) converging early.
// ---------------------------------------------------------------------------
function w111() {
  const nodes = [
    ...prologueNodes(111, ["alpha", "beta", "gamma", "delta"], {
      branchA: 5,
      branchB: 6,
    }),
    deepAgent("d1", "Boundary Stage 1", 7),
    deepAgent("d2", "Boundary Stage 2", 8),
    {
      id: "iter",
      initialWidth: 640,
      initialHeight: 340,
      data: {
        kind: "iteration",
        title: "Boundary Iteration (items == maxIterations)",
        iterationConfig: {
          iteratorSelector: ["start", "items"],
          collectSelector: ["iter-agent", "output"],
          errorStrategy: "continue",
          maxIterations: 4,
        },
      },
    },
    {
      id: "iter-agent",
      parentId: "iter",
      data: {
        kind: "agent",
        title: "Iteration Deep Worker",
        agentConfig: (() => {
          const config = agentConfiguration(iterationPrompt("iter"));
          config.promptInactivity = "wait";
          return config;
        })(),
      },
    },
    deepAgent("d3", "Post-Boundary Audit", 9),
    ...loopNodes("loop", { rounds: 3, maxIterations: 100 }),
    deepAgent("d4", "Ceiling Verification", 11),
    deepAgent("d5", "Final Boundary Report", 12),
    deepAgent("d6", "Boundary Cross-Check", 6),
    outputNode([
      { name: "branch", variableSelector: ["agg", "output"] },
      { name: "iterated", variableSelector: ["iter", "output"] },
      { name: "loop_result", variableSelector: ["loop", "result"] },
      { name: "final", variableSelector: ["d6", "output"] },
    ]),
  ];
  const edges = [
    ...prologueEdges,
    { source: "agg", target: "d1" },
    { source: "d1", target: "d2" },
    { source: "d2", target: "iter" },
    { source: "iter", sourceHandle: "iteration-entry", target: "iter-agent" },
    { source: "iter", target: "d3" },
    { source: "d3", target: "loop" },
    { source: "loop-start", target: "loop-agent" },
    { source: "loop", target: "d4" },
    { source: "d4", target: "d5" },
    { source: "d5", target: "d6" },
    { source: "d6", target: "out" },
  ];
  return finalize(
    111,
    "wf-111-superlong-boundary-config",
    "Super-Long: Configuration Boundaries",
    "Iteration source length exactly equals maxIterations; loop maxIterations at the 100 ceiling converging at round 3.",
    { nodes, edges },
  );
}

// ---------------------------------------------------------------------------
// W112: kitchen sink — one condition, one aggregator, two iterations, two loops, wide+deep.
// ---------------------------------------------------------------------------
function w112() {
  const nodes = [
    ...prologueNodes(112, ["alpha", "beta", "gamma", "delta"], {
      branchA: 6,
      branchB: 7,
    }),
    deepAgent("k1", "Sink Stage 1", 8),
    deepAgent("p1", "Sink Parallel 1", 9),
    deepAgent("p2", "Sink Parallel 2", 10),
    deepAgent("join", "Sink Join", 11),
    ...iterationNodes("iter", { strategy: "continue" }),
    deepAgent("k2", "Sink Stage 2", 1),
    ...iterationNodes("iter2", {
      sourceVar: "items2",
      strategy: "fail",
      maxIterations: 10,
    }),
    ...loopNodes("loop1", {
      rounds: 3,
      maxIterations: 5,
      variable: "sink_one",
    }),
    deepAgent("k3", "Sink Stage 3", 4),
    ...loopNodes("loop2", {
      rounds: 4,
      maxIterations: 6,
      variable: "sink_two",
    }),
    deepAgent("k4", "Final Sink Verdict", 6),
    outputNode([
      { name: "branch", variableSelector: ["agg", "output"] },
      { name: "join", variableSelector: ["join", "output"] },
      { name: "iterated", variableSelector: ["iter", "output"] },
      { name: "iterated2", variableSelector: ["iter2", "output"] },
      { name: "loop1_result", variableSelector: ["loop1", "result"] },
      { name: "loop2_result", variableSelector: ["loop2", "result"] },
      { name: "final", variableSelector: ["k4", "output"] },
    ]),
  ];
  const edges = [
    ...prologueEdges,
    { source: "agg", target: "k1" },
    { source: "k1", target: "p1" },
    { source: "k1", target: "p2" },
    { source: "p1", target: "join" },
    { source: "p2", target: "join" },
    { source: "join", target: "iter" },
    { source: "iter", sourceHandle: "iteration-entry", target: "iter-agent" },
    { source: "iter", target: "k2" },
    { source: "k2", target: "iter2" },
    { source: "iter2", sourceHandle: "iteration-entry", target: "iter2-agent" },
    { source: "iter2", target: "loop1" },
    { source: "loop1-start", target: "loop1-agent" },
    { source: "loop1", target: "k3" },
    { source: "k3", target: "loop2" },
    { source: "loop2-start", target: "loop2-agent" },
    { source: "loop2", target: "k4" },
    { source: "k4", target: "out" },
  ];
  return finalize(
    112,
    "wf-112-superlong-kitchen-sink",
    "Super-Long: Kitchen Sink (2 iterations, 2 loops)",
    "One condition, one aggregator, two iterations (continue+fail), two loops, parallel fan-out with join, 12+ deep agents.",
    { nodes, edges },
  );
}

/** Generates the eleven super-long endurance workflows W102-W112. */
export function generateSuperLongWorkflows() {
  return [
    w102(),
    w103(),
    w104(),
    w105(),
    w106(),
    w107(),
    w108(),
    w109(),
    w110(),
    w111(),
    w112(),
  ];
}
