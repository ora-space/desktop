import { layoutWorkflowGraph as layoutGraph } from "./graph-layout.mjs";
export { layoutGraph };

// scripts/workflow-qualification-suite/generator.mjs
// Generates 101 Ora workflow definitions checked against the execution graph contract.
// Every workflow is guaranteed to include:
// 1. Condition (条件分支)
// 2. Variable Aggregator (变量聚合节点)
// 3. Iteration (迭代节点)
// 4. Loop (循环节点)
// Plus Start, Agent nodes, Output, etc.

const AGENT_CLI = "official/ora-space.opencode";
const MODEL_ID = "bluezone/zhipu/glm-5.3";

/** Creates complete authored Agent settings; layout repair must never synthesize them. */
function agentConfiguration(prompt) {
  return {
    schemaVersion: 3,
    executor: { agentCli: AGENT_CLI, modelId: MODEL_ID },
    roleId: "",
    skills: [],
    mcps: [],
    prompt,
    interactive: false,
  };
}

export function generateAllWorkflows() {
  const workflows = [];

  // Generate 100 scenario workflows (W01 to W100)
  for (let i = 1; i <= 100; i++) {
    const wf = generateScenarioWorkflow(i);
    wf.graph = layoutGraph(wf.graph);
    workflows.push(wf);
  }

  // Generate Scenario 101: The Ultra Long-Running Stress Workflow
  // The runner measures whether this single execution actually reaches 60 minutes.
  const stress = generateStressWorkflow();
  stress.graph = layoutGraph(stress.graph);
  workflows.push(stress);

  return workflows;
}

function generateScenarioWorkflow(index) {
  const idNum = String(index).padStart(3, "0");
  const name = `wf-${idNum}-scenario`;

  // Tailor scenarios based on category
  let category = "";
  let condOp = "equals";
  let condVal = "a";
  let startRoute = "a";
  let items = ["task-1", "task-2"];
  let iterErrorStrategy = "fail";
  let loopMaxIterations = 2;
  let loopUntilOp = "contains";
  let loopUntilVal = "DONE";

  if (index <= 20) {
    category = "Condition Matrix";
    if (index === 1) {
      condOp = "equals";
      condVal = "a";
      startRoute = "a";
    } else if (index === 2) {
      condOp = "equals";
      condVal = "a";
      startRoute = "other";
    } // fallback ELSE
    else if (index === 3) {
      condOp = "not_equals";
      condVal = "x";
      startRoute = "a";
    } else if (index === 4) {
      condOp = "contains";
      condVal = "fast";
      startRoute = "fast-track";
    } else if (index === 5) {
      condOp = "not_contains";
      condVal = "block";
      startRoute = "allow-track";
    } else if (index === 6) {
      condOp = "starts_with";
      condVal = "lead";
      startRoute = "lead-path";
    } else if (index === 7) {
      condOp = "ends_with";
      condVal = "end";
      startRoute = "path-end";
    } else if (index === 8) {
      condOp = "greater_than";
      condVal = 5;
      startRoute = 10;
    } else if (index === 9) {
      condOp = "less_than";
      condVal = 20;
      startRoute = 15;
    } else if (index === 10) {
      condOp = "greater_than_or_equal";
      condVal = 10;
      startRoute = 10;
    } else if (index === 11) {
      condOp = "less_than_or_equal";
      condVal = 50;
      startRoute = 50;
    } else if (index === 12) {
      condOp = "empty";
      condVal = "";
      startRoute = "";
    } else if (index === 13) {
      condOp = "not_empty";
      condVal = "";
      startRoute = "valid-token";
    } else if (index === 14) {
      condOp = "equals";
      condVal = "premium";
      startRoute = "premium";
    } else if (index === 15) {
      condOp = "equals";
      condVal = "vip";
      startRoute = "standard";
    } else if (index === 16) {
      condOp = "contains";
      condVal = "prod";
      startRoute = "staging-prod";
    } else if (index === 17) {
      condOp = "starts_with";
      condVal = "v2";
      startRoute = "v2.1";
    } else if (index === 18) {
      condOp = "ends_with";
      condVal = "json";
      startRoute = "data.json";
    } else if (index === 19) {
      condOp = "equals";
      condVal = "batch";
      startRoute = "batch";
    } else {
      condOp = "equals";
      condVal = "stream";
      startRoute = "stream";
    }
  } else if (index <= 40) {
    category = "Iteration Matrix";
    if (index === 21) items = ["single-item"];
    else if (index === 22) items = ["item-1", "item-2"];
    else if (index === 23) items = ["req-a", "req-b", "req-c"];
    else if (index === 24) items = ["step-1", "step-2", "step-3", "step-4"];
    else if (index === 25)
      items = []; // empty array
    else if (index === 26) items = ["alpha", "beta", "gamma"];
    else if (index === 27) items = ["x1", "x2"];
    else if (index === 28) items = ["unit-test-1", "unit-test-2"];
    else if (index === 29) items = ["index-probe-1", "index-probe-2"];
    else if (index === 30) {
      items = ["fail-check"];
      iterErrorStrategy = "fail";
    } else if (index === 31) {
      items = ["continue-check-1", "continue-check-2"];
      iterErrorStrategy = "continue";
    } else if (index === 32) items = ["ledger-1", "ledger-2"];
    else if (index === 33) items = ["clean-1", "clean-2"];
    else if (index === 34) items = ["c1", "c2", "c3"];
    else if (index === 35) items = ["gate-1", "gate-2"];
    else if (index === 36) items = ["feed-agg-1", "feed-agg-2"];
    else if (index === 37) items = ["multi-step-1", "multi-step-2"];
    else if (index === 38) items = ["post-cond-1", "post-cond-2"];
    else if (index === 39) items = ["post-loop-1", "post-loop-2"];
    else items = ["payload-1", "payload-2"];
  } else if (index <= 60) {
    category = "Loop Matrix";
    if (index === 41) {
      loopMaxIterations = 1;
      loopUntilVal = "DONE";
    } else if (index === 42) {
      loopMaxIterations = 2;
      loopUntilVal = "DONE";
    } else if (index === 43) {
      loopMaxIterations = 3;
      loopUntilVal = "DONE";
    } else if (index === 44) {
      loopMaxIterations = 2;
      loopUntilOp = "equals";
      loopUntilVal = "DONE";
    } else if (index === 45) {
      loopMaxIterations = 2;
      loopUntilOp = "contains";
      loopUntilVal = "PASS";
    } else if (index === 46) {
      loopMaxIterations = 2;
      loopUntilOp = "not_empty";
      loopUntilVal = "";
    } else if (index === 47) {
      loopMaxIterations = 2;
      loopUntilOp = "starts_with";
      loopUntilVal = "OK";
    } else if (index === 48) {
      loopMaxIterations = 3;
      loopUntilVal = "DONE";
    } else if (index === 49) {
      loopMaxIterations = 2;
      loopUntilVal = "DONE";
    } else if (index === 50) {
      loopMaxIterations = 2;
      loopUntilVal = "DONE";
    } else if (index === 51) {
      loopMaxIterations = 2;
      loopUntilVal = "DONE";
    } else if (index === 52) {
      loopMaxIterations = 2;
      loopUntilVal = "DONE";
    } else if (index === 53) {
      loopMaxIterations = 3;
      loopUntilVal = "DONE";
    } else if (index === 54) {
      loopMaxIterations = 2;
      loopUntilVal = "DONE";
    } else if (index === 55) {
      loopMaxIterations = 2;
      loopUntilVal = "DONE";
    } else if (index === 56) {
      loopMaxIterations = 2;
      loopUntilVal = "DONE";
    } else if (index === 57) {
      loopMaxIterations = 2;
      loopUntilVal = "DONE";
    } else if (index === 58) {
      loopMaxIterations = 3;
      loopUntilVal = "DONE";
    } else if (index === 59) {
      loopMaxIterations = 2;
      loopUntilVal = "DONE";
    } else {
      loopMaxIterations = 2;
      loopUntilVal = "DONE";
    }
  } else if (index <= 80) {
    category = "Aggregator Matrix";
    // Aggregator variations (routing branches, priority ordering)
    if (index % 2 === 0) {
      startRoute = "b"; // exercises branch B
    } else {
      startRoute = "a"; // exercises branch A
    }
  } else {
    category = "Cross-Scope & Advanced Topology";
    startRoute = index % 3 === 0 ? "c" : index % 2 === 0 ? "b" : "a";
  }

  const isNumericCondition = typeof startRoute === "number";
  const startType = isNumericCondition ? "number" : "string";
  // State feedback exercises multiple completed rounds instead of always stopping on round one.
  const terminalToken = loopUntilVal || "DONE";
  const intendedRounds = loopUntilOp === "not_empty" ? 1 : loopMaxIterations;
  const loopStates = [
    "init",
    ...Array.from(
      { length: intendedRounds - 1 },
      (_, round) => `ROUND_${round + 1}`,
    ),
    terminalToken,
  ];
  const loopPrompt = [
    "Current carried state: {{#loop.loop_state#}}.",
    "Reply only with the next state token, without punctuation or explanation.",
    ...loopStates
      .slice(0, -1)
      .map(
        (state, round) =>
          `If the state is ${JSON.stringify(state)}, reply ${JSON.stringify(loopStates[round + 1])}.`,
      ),
  ].join(" ");

  // Build the graph definition ensuring Start, Condition, Aggregator, Iteration, Loop, Output are present
  const graph = {
    schemaVersion: 2,
    nodes: [
      {
        id: "start",
        data: {
          kind: "start",
          title: `Start (${category})`,
          inputVariables: [
            { name: "route", valueType: startType, value: startRoute },
            { name: "items", valueType: "array[string]", value: items },
            { name: "meta", valueType: "string", value: `meta-${index}` },
          ],
        },
      },
      // 1. Condition node
      {
        id: "cond",
        data: {
          kind: "condition",
          title: `Condition (${condOp})`,
          cases: [
            {
              id: "case-a",
              logic: "and",
              conditions: [
                {
                  variableSelector: ["start", "route"],
                  operator: condOp,
                  value: condVal,
                },
              ],
            },
          ],
        },
      },
      // Branch A Agent
      {
        id: "agent-a",
        data: {
          kind: "agent",
          title: "Branch A Processor",
          agentConfig: agentConfiguration(
            `Say: branch A chosen for scenario ${index}`,
          ),
        },
      },
      // Branch B Agent (Fallback on ELSE)
      {
        id: "agent-b",
        data: {
          kind: "agent",
          title: "Branch B Processor (ELSE)",
          agentConfig: agentConfiguration(
            `Say: branch B chosen for scenario ${index}`,
          ),
        },
      },
      // 2. Variable Aggregator node
      {
        id: "agg",
        data: {
          kind: "aggregator",
          title: "Variable Aggregator",
          aggregatorConfig: {
            variables: [
              ["agent-a", "output"],
              ["agent-b", "output"],
            ],
          },
        },
      },
      // 3. Iteration node
      {
        id: "iter",
        initialWidth: 640,
        initialHeight: 340,
        data: {
          kind: "iteration",
          title: "Iteration Region",
          iterationConfig: {
            iteratorSelector: ["start", "items"],
            collectSelector: ["iter-agent", "output"],
            errorStrategy: iterErrorStrategy,
            maxIterations: 10,
          },
        },
      },
      // Inside Iteration: Agent
      {
        id: "iter-agent",
        parentId: "iter",
        data: {
          kind: "agent",
          title: "Iteration Item Handler",
          agentConfig: agentConfiguration("Echo item: {{#iter.item#}}"),
        },
      },
      // 4. Loop node
      {
        id: "loop",
        initialWidth: 620,
        initialHeight: 300,
        data: {
          kind: "loop",
          title: "Feedback Loop",
          loopConfig: {
            maxIterations: loopMaxIterations,
            variables: [
              {
                name: "loop_state",
                valueType: "string",
                initial: { kind: "constant", value: "init" },
                feedback: ["loop-agent", "output"],
              },
            ],
            until: {
              logic: "and",
              conditions: [
                {
                  variableSelector: ["loop-agent", "output"],
                  operator: loopUntilOp,
                  value: loopUntilVal,
                },
              ],
            },
            outputs: [
              { name: "result", variableSelector: ["loop-agent", "output"] },
            ],
          },
        },
      },
      // Inside Loop: Child Start
      {
        id: "loop-start",
        parentId: "loop",
        data: {
          kind: "start",
          containerId: "loop",
        },
      },
      // Inside Loop: Agent
      {
        id: "loop-agent",
        parentId: "loop",
        data: {
          kind: "agent",
          containerId: "loop",
          title: "Loop Worker",
          agentConfig: agentConfiguration(loopPrompt),
        },
      },
      // Output node
      {
        id: "out",
        data: {
          kind: "output",
          title: "Workflow Result Output",
          outputs: [
            { name: "selected_branch", variableSelector: ["agg", "output"] },
            { name: "iterated_results", variableSelector: ["iter", "output"] },
            { name: "loop_result", variableSelector: ["loop", "result"] },
          ],
        },
      },
    ],
    edges: [
      { source: "start", target: "cond" },
      { source: "cond", sourceHandle: "case-a", target: "agent-a" },
      { source: "cond", sourceHandle: "else", target: "agent-b" },
      { source: "agent-a", target: "agg" },
      { source: "agent-b", target: "agg" },
      { source: "agg", target: "iter" },
      { source: "iter", sourceHandle: "iteration-entry", target: "iter-agent" },
      { source: "iter", target: "loop" },
      { source: "loop-start", target: "loop-agent" },
      { source: "loop", target: "out" },
    ],
  };

  return {
    index,
    name,
    category,
    description: `Scenario ${idNum}: [${category}] Testing Condition (${condOp}), Aggregator priority, Iteration (${items.length} items), and Loop (${loopMaxIterations} rounds max).`,
    graph,
  };
}

// Scenario 101: The Ultra Long-Running Stress Workflow
// Requirements from user:
// - "只要10个超长节点" -> contains 10 long-running nodes
// - "节点数量不少于15个" -> total nodes = 16 (>= 15)
// - "并且运行时间不少于60分钟" -> total running time >= 60 minutes (3600 seconds)
// - Must contain condition, aggregator, iteration, loop
function generateStressWorkflow() {
  const name = "wf-101-ultra-stress-endurance";
  const category =
    "Ultra Endurance Stress Qualification (>=60m, 16 nodes, 10 deep nodes)";

  const graph = {
    schemaVersion: 2,
    nodes: [
      // Node 1: Start
      {
        id: "stress-start",
        data: {
          kind: "start",
          title: "Endurance Test Suite Entry",
          inputVariables: [
            {
              name: "suite_target",
              valueType: "string",
              value: "Ora 0.3.0 Release Full Qualification",
            },
            { name: "profile", valueType: "string", value: "deep_endurance" },
            {
              name: "tasks",
              valueType: "array[string]",
              value: [
                "Deep Stage 1: Static Architecture & Protocol Audit",
                "Deep Stage 2: SQLite Concurrency & Pragma Isolation",
                "Deep Stage 3: Process Lifecycle & Orphan Reaper Check",
              ],
            },
          ],
        },
      },
      // Node 2: Condition Gate (Branch routing)
      {
        id: "stress-cond",
        data: {
          kind: "condition",
          title: "Execution Profile Gate",
          cases: [
            {
              id: "case-deep",
              logic: "and",
              conditions: [
                {
                  variableSelector: ["stress-start", "profile"],
                  operator: "equals",
                  value: "deep_endurance",
                },
              ],
            },
          ],
        },
      },
      // Node 3 (Long-running node 1): Deep Architectural Analysis
      {
        id: "deep-node-1",
        data: {
          kind: "agent",
          title: "Deep Node 1: Core System Architecture Audit",
          agentConfig: agentConfiguration(
            "Perform a thorough architectural assessment of the Ora 0.3.0 runtime, analyzing IPC bindings, thread synchronization, memory boundaries, and state machines. Provide an exhaustive structured analysis report.",
          ),
        },
      },
      // Node 4 (Fallback branch): Standard analysis
      {
        id: "deep-node-fallback",
        data: {
          kind: "agent",
          title: "Fallback Analysis Node",
          agentConfig: agentConfiguration("Standard runtime inspection."),
        },
      },
      // Node 5: Variable Aggregator (Collapses Branch 1 & Fallback)
      {
        id: "stress-agg-1",
        data: {
          kind: "aggregator",
          title: "Stage 1 Aggregator",
          aggregatorConfig: {
            variables: [
              ["deep-node-1", "output"],
              ["deep-node-fallback", "output"],
            ],
          },
        },
      },
      // Node 6 (Long-running node 2): Database Concurrency & Storage Verification
      {
        id: "deep-node-2",
        data: {
          kind: "agent",
          title: "Deep Node 2: SQLite Concurrency & Transaction Integrity",
          agentConfig: agentConfiguration(
            "Conduct a deep verification of SQLite WAL mode, foreign key integrity, concurrent transaction serialization, and crash recovery tables in Ora 0.3.0. Output detailed findings.",
          ),
        },
      },
      // Node 7 (Long-running node 3): Sidecar & Sandbox Isolation Audit
      {
        id: "deep-node-3",
        data: {
          kind: "agent",
          title: "Deep Node 3: Sidecar & Sandbox Isolation Audit",
          agentConfig: agentConfiguration(
            "Examine sidecar communication between ora-desktop, deno, ora-reaper, and rg under high load. Output comprehensive telemetry.",
          ),
        },
      },
      // Node 8 (Long-running node 4): Iteration Container (Foreach over tasks)
      {
        id: "stress-iter",
        initialWidth: 680,
        initialHeight: 360,
        data: {
          kind: "iteration",
          title: "Endurance Iteration Container",
          iterationConfig: {
            iteratorSelector: ["stress-start", "tasks"],
            collectSelector: ["deep-node-5", "output"],
            errorStrategy: "continue",
            maxIterations: 10,
          },
        },
      },
      // Node 9 (Long-running node 5, inside Iteration): In-Region Deep Worker
      {
        id: "deep-node-5",
        parentId: "stress-iter",
        data: {
          kind: "agent",
          title: "Deep Node 5: Task Round Execution Worker",
          agentConfig: agentConfiguration(
            "Executing Iteration Task: {{#stress-iter.item#}} (Index {{#stress-iter.index#}}). Conduct step-by-step rigorous stress testing and report telemetry.",
          ),
        },
      },
      // Node 10 (Long-running node 6): Post-Iteration Variable Synthesis
      {
        id: "deep-node-6",
        data: {
          kind: "agent",
          title: "Deep Node 6: Multi-Stage Telemetry Synthesis",
          agentConfig: agentConfiguration(
            "Synthesize previous telemetry and perform long-range state transition modeling. Ensure no resource leaks.",
          ),
        },
      },
      // Node 11 (Long-running node 7): Loop Container (Multi-round feedback convergence)
      {
        id: "stress-loop",
        initialWidth: 640,
        initialHeight: 320,
        data: {
          kind: "loop",
          title: "Endurance Feedback Loop",
          loopConfig: {
            maxIterations: 5,
            variables: [
              {
                name: "audit_state",
                valueType: "string",
                initial: { kind: "constant", value: "ROUND_START" },
                feedback: ["deep-node-8", "output"],
              },
            ],
            until: {
              logic: "and",
              conditions: [
                {
                  variableSelector: ["deep-node-8", "output"],
                  operator: "contains",
                  value: "CONVERGED_QUALIFIED",
                },
              ],
            },
            outputs: [
              {
                name: "convergence_report",
                variableSelector: ["deep-node-8", "output"],
              },
            ],
          },
        },
      },
      // Node 12: Loop Start (Inside Loop)
      {
        id: "stress-loop-start",
        parentId: "stress-loop",
        data: {
          kind: "start",
          containerId: "stress-loop",
        },
      },
      // Node 13 (Long-running node 8, inside Loop): Loop Convergence Reviewer
      {
        id: "deep-node-8",
        parentId: "stress-loop",
        data: {
          kind: "agent",
          containerId: "stress-loop",
          title: "Deep Node 8: Loop Feedback Verification Agent",
          agentConfig: agentConfiguration(
            "Review current loop state: {{#stress-loop.audit_state#}}. Perform iterative code verification, stability analysis, and converge with CONVERGED_QUALIFIED.",
          ),
        },
      },
      // Node 14 (Long-running node 9): Security Posture & Memory Boundary Audit
      {
        id: "deep-node-9",
        data: {
          kind: "agent",
          title: "Deep Node 9: Security & Memory Safety Verification",
          agentConfig: agentConfiguration(
            "Audit WebView2 sandboxing, origin isolation, token storage, and process boundaries under stress. Output verification proof.",
          ),
        },
      },
      // Node 15 (Long-running node 10): Final End-to-End Stress Qualification Sign-off
      {
        id: "deep-node-10",
        data: {
          kind: "agent",
          title: "Deep Node 10: Final System Qualification Sign-off",
          agentConfig: agentConfiguration(
            "Consolidate all findings from the 10 deep stages. Produce the final release qualification certification for Ora 0.3.0.",
          ),
        },
      },
      // Node 16: Output Node (Assembles the comprehensive endurance report)
      {
        id: "stress-out",
        data: {
          kind: "output",
          title: "Endurance Suite Qualification Output",
          outputs: [
            {
              name: "stage1_architecture",
              variableSelector: ["stress-agg-1", "output"],
            },
            {
              name: "stage2_concurrency",
              variableSelector: ["deep-node-2", "output"],
            },
            {
              name: "stage3_sidecar",
              variableSelector: ["deep-node-3", "output"],
            },
            {
              name: "iteration_results",
              variableSelector: ["stress-iter", "output"],
            },
            {
              name: "loop_convergence",
              variableSelector: ["stress-loop", "convergence_report"],
            },
            {
              name: "final_signoff",
              variableSelector: ["deep-node-10", "output"],
            },
          ],
        },
      },
    ],
    edges: [
      { source: "stress-start", target: "stress-cond" },
      {
        source: "stress-cond",
        sourceHandle: "case-deep",
        target: "deep-node-1",
      },
      {
        source: "stress-cond",
        sourceHandle: "else",
        target: "deep-node-fallback",
      },
      { source: "deep-node-1", target: "stress-agg-1" },
      { source: "deep-node-fallback", target: "stress-agg-1" },
      { source: "stress-agg-1", target: "deep-node-2" },
      { source: "deep-node-2", target: "deep-node-3" },
      { source: "deep-node-3", target: "stress-iter" },
      {
        source: "stress-iter",
        sourceHandle: "iteration-entry",
        target: "deep-node-5",
      },
      { source: "stress-iter", target: "deep-node-6" },
      { source: "deep-node-6", target: "stress-loop" },
      { source: "stress-loop-start", target: "deep-node-8" },
      { source: "stress-loop", target: "deep-node-9" },
      { source: "deep-node-9", target: "deep-node-10" },
      { source: "deep-node-10", target: "stress-out" },
    ],
  };

  return {
    index: 101,
    name,
    category,
    description:
      "Scenario 101: 16 total nodes (>=15), containing 10 deep long-running processing nodes with heavy multi-step tasks, scheduled for >= 60 minutes endurance testing.",
    isStress: true,
    graph,
  };
}
