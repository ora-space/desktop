import assert from "node:assert/strict";
import test from "node:test";
import { analyzeWorkflows } from "./analysis.mjs";
import { generateAllWorkflows } from "./generator.mjs";

test("validates every generated graph through the production analysis operation without creating data", async () => {
  const workflows = generateAllWorkflows();
  const calls = [];
  const analyses = await analyzeWorkflows(
    {
      invoke(command, request) {
        calls.push({ command, request });
        return { unusedNodeIds: [] };
      },
    },
    workflows,
  );
  assert.equal(workflows.length, 101);
  assert.deepEqual(
    calls,
    workflows.map((workflow) => ({
      command: "analyze_workflow",
      request: { graph: JSON.stringify(workflow.graph) },
    })),
  );
  assert.ok(analyses.every((analysis) => analysis.valid));
});

test("one rejected graph records a failure while the remaining graphs are still inspected", async () => {
  const workflows = [
    { index: 1, name: "bad", graph: {} },
    { index: 2, name: "good", graph: {} },
  ];
  let calls = 0;
  const analyses = await analyzeWorkflows(
    {
      invoke() {
        if (calls++ === 0) throw new Error("Invalid ownership");
        return { unusedNodeIds: ["spare"] };
      },
    },
    workflows,
  );
  assert.deepEqual(analyses, [
    { index: 1, name: "bad", valid: false, error: "Invalid ownership" },
    { index: 2, name: "good", valid: true, unusedNodeIds: ["spare"] },
  ]);
});

test("malformed analysis responses are failures", async () => {
  const analyses = await analyzeWorkflows({ invoke: () => ({}) }, [
    { index: 1, name: "bad", graph: {} },
  ]);
  assert.equal(analyses[0].valid, false);
  assert.match(analyses[0].error, /unusedNodeIds/);
});
