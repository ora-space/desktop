import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { main } from "./runner.mjs";
import { main as validate } from "./validate_sample.mjs";

/** Keeps durable-report assertions in a dedicated temporary directory. */
function outputRoot(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "ora-qualification-"));
  assert.equal(path.dirname(root), path.resolve(os.tmpdir()));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  t.mock.method(console, "log", () => {});
  return root;
}

const workflows = [
  { index: 1, name: "first", graph: { nodes: [] } },
  { index: 101, name: "endurance", isStress: true, graph: { nodes: [] } },
];

/** Rejects unexpected operations instead of returning a synthetic success for mutations. */
function sessionFixture({ rejectedAnalysis, workspaceKind = "isolated" } = {}) {
  const calls = [];
  let closeCount = 0;
  return {
    calls,
    get closeCount() {
      return closeCount;
    },
    close() {
      closeCount++;
    },
    invoke(command, request) {
      calls.push({ command, request });
      if (command === "list_workspaces")
        return {
          workspaces: [
            { id: "one", lifecycle: "active", kind: workspaceKind },
            { id: "two", lifecycle: "active", kind: workspaceKind },
          ],
        };
      if (command === "analyze_workflow") {
        if (rejectedAnalysis) throw new Error("Invalid graph");
        return { unusedNodeIds: [] };
      }
      throw new Error(`Unexpected operation: ${command}`);
    },
  };
}

test("an explicit workspace is required before connecting, generating, or writing", async (t) => {
  const root = outputRoot(t);
  let calls = 0;
  await assert.rejects(
    main({
      env: {},
      outputRoot: root,
      connect: () => {
        calls++;
      },
      generate: () => {
        calls++;
      },
    }),
    /ORA_QUALIFICATION_WORKSPACE_ID/,
  );
  assert.equal(calls, 0);
  assert.deepEqual(fs.readdirSync(root), []);
});

test("a rejected graph leaves a failed report and creates no definitions or runs", async (t) => {
  const root = outputRoot(t);
  const session = sessionFixture({ rejectedAnalysis: true });
  const result = await main({
    env: { ORA_QUALIFICATION_WORKSPACE_ID: "one" },
    outputRoot: root,
    connect: () => session,
    generate: () => workflows,
  });
  assert.equal(result.passed, false);
  assert.equal(session.closeCount, 1);
  assert.deepEqual(
    session.calls.map((call) => call.command),
    ["list_workspaces", "analyze_workflow", "analyze_workflow"],
  );
  const evidence = JSON.parse(fs.readFileSync(result.resultsFile, "utf8"));
  assert.equal(evidence.analyses.length, 2);
  assert.deepEqual(evidence.results, []);
  assert.match(
    fs.readFileSync(result.reportFile, "utf8"),
    /FAIL \/ INCOMPLETE/,
  );
});

test("an explicit ID cannot accidentally qualify an ordinary main workspace", async (t) => {
  const session = sessionFixture({ workspaceKind: "main" });
  const result = await main({
    env: { ORA_QUALIFICATION_WORKSPACE_ID: "one" },
    outputRoot: outputRoot(t),
    connect: () => session,
    generate: () => workflows,
  });
  assert.equal(result.passed, false);
  assert.deepEqual(
    session.calls.map((call) => call.command),
    ["list_workspaces"],
  );
  assert.equal(session.closeCount, 1);
});

test("a changed build or workspace always executes afresh rather than trusting stale results", async (t) => {
  const root = outputRoot(t);
  fs.writeFileSync(
    path.join(root, "qualification_results.json"),
    JSON.stringify([{ index: 1, qualified: true }]),
  );
  const executed = [];
  const execute = (_session, workflow, workspaceId) => {
    executed.push([workflow.index, workspaceId]);
    return {
      index: workflow.index,
      qualified: true,
      cleanupComplete: true,
      status: "succeeded",
    };
  };
  const outputs = [];
  for (const [workspaceId, build] of [
    ["one", "build-a"],
    ["two", "build-b"],
  ]) {
    const session = sessionFixture();
    outputs.push(
      await main({
        env: {
          ORA_QUALIFICATION_WORKSPACE_ID: workspaceId,
          ORA_QUALIFICATION_BUILD_ID: build,
        },
        outputRoot: root,
        connect: () => session,
        generate: () => workflows,
        scenario: execute,
        endurance: execute,
      }),
    );
    assert.equal(session.closeCount, 1);
  }
  assert.deepEqual(executed, [
    [1, "one"],
    [101, "one"],
    [1, "two"],
    [101, "two"],
  ]);
  assert.notEqual(outputs[0].outputDirectory, outputs[1].outputDirectory);
  assert.equal(
    JSON.parse(fs.readFileSync(outputs[1].resultsFile, "utf8")).context.build
      .label,
    "build-b",
  );
});

test("cleanup failure halts subsequent agents and cannot disappear from the report", async (t) => {
  const session = sessionFixture();
  let executionCount = 0;
  const execute = (_session, workflow) => {
    executionCount++;
    return {
      index: workflow.index,
      status: "timeout",
      qualified: false,
      cleanupComplete: false,
      cleanupErrors: ["Cancellation did not settle run"],
    };
  };
  const result = await main({
    env: { ORA_QUALIFICATION_WORKSPACE_ID: "one" },
    outputRoot: outputRoot(t),
    connect: () => session,
    generate: () => workflows,
    scenario: execute,
    endurance: execute,
  });
  assert.equal(result.passed, false);
  assert.equal(executionCount, 1);
  assert.equal(session.closeCount, 1);
  assert.match(
    fs.readFileSync(result.reportFile, "utf8"),
    /Cancellation did not settle/,
  );
});

test("the validation entry point inspects all 101 graphs without creating user data", async (t) => {
  t.mock.method(console, "log", () => {});
  const session = sessionFixture();
  const result = await validate({ connect: () => session });
  assert.equal(result.passed, true);
  assert.equal(result.analyses.length, 101);
  assert.ok(session.calls.every((call) => call.command === "analyze_workflow"));
  assert.equal(session.closeCount, 1);
});

test("validation reports rejection and closes its transport", async (t) => {
  t.mock.method(console, "log", () => {});
  const session = sessionFixture({ rejectedAnalysis: true });
  const result = await validate({
    connect: () => session,
    generate: () => workflows,
  });
  assert.equal(result.passed, false);
  assert.equal(session.closeCount, 1);
});
