import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import vm from "node:vm";

const directory = new URL("./", import.meta.url);
const graph = {
  nodes: Array.from({ length: 16 }, (_, index) => ({
    id: `node-${index}`,
    data: { kind: ["condition", "aggregator", "iteration", "loop"][index % 4] },
  })),
};
const workflow = { index: 101, name: "stress", isStress: true, graph };

/** Runs the production functions in a VM so timers cannot reach a live app. */
function loadRunner(clock, session) {
  const source = fs
    .readFileSync(new URL("execution.mjs", directory), "utf8")
    .replace(/^export /gm, "");
  const context = vm.createContext({
    Date: { now: () => clock.value },
    setTimeout(callback, milliseconds) {
      clock.value += clock.step ?? milliseconds;
      callback();
    },
    log() {},
    fs: { writeFileSync() {} },
    path: {
      join() {
        return "unused";
      },
    },
    OUTPUT_DIR: "unused",
    session,
  });
  return vm.runInContext(
    `${source}\n({ runScenario, runEnduranceStress, runSuperLong });`,
    context,
  );
}

/** Supplies real command shapes while retaining every call for lifecycle assertions. */
function createSession(status = "succeeded") {
  const calls = [];
  const detail = {
    run: {
      id: "run-1",
      status,
      output: "{}",
      startedAt: 0,
      finishedAt: 3_600_000,
    },
    nodes: graph.nodes.map((node) => ({
      nodeId: node.id,
      nodeType: node.data.kind,
      status: "succeeded",
    })),
  };
  return {
    calls,
    invoke(command, request) {
      calls.push({ command, request });
      if (command === "create_workflow") {
        return { workflow: { id: "workflow-1" } };
      }
      if (command === "create_workflow_run") return { run: { id: "run-1" } };
      if (command === "get_workflow_run") return detail;
      if (command === "cancel_workflow_run") detail.run.status = "cancelled";
      return {};
    },
  };
}

test("the one-hour endurance regression returns the actual final workflow and run IDs", async () => {
  const clock = { value: 0, step: 3_600_000 };
  const session = createSession();
  const runner = loadRunner(clock, session);
  const result = await runner.runEnduranceStress(
    session,
    workflow,
    "disposable-workspace",
    {
      now: () => clock.value,
      sleep: () => {
        clock.value += clock.step;
      },
    },
  );
  assert.equal(result.workflowId, "workflow-1");
  assert.equal(result.runId, "run-1");
  assert.equal(result.qualified, true);
});

test("a timed-out active scenario is cancelled and settled before returning", async () => {
  const clock = { value: 0 };
  const session = createSession("running");
  const runner = loadRunner(clock, session);
  const result = await runner.runScenario(
    session,
    workflow,
    "disposable-workspace",
    {
      timeoutMs: 50,
      pollIntervalMs: 10,
      now: () => clock.value,
      sleep: (milliseconds) => {
        clock.value += milliseconds;
      },
    },
  );
  assert.equal(result.status, "timeout");
  assert.equal(result.qualified, false);
  const cancellation = session.calls.findIndex(
    (call) => call.command === "cancel_workflow_run",
  );
  assert.ok(cancellation >= 0);
  assert.ok(
    session.calls
      .slice(cancellation + 1)
      .some((call) => call.command === "get_workflow_run"),
  );
});

test("a super-long run must meet the node, deep-node, and duration class bars", async () => {
  const clock = { value: 0, step: 20 * 60_000 };
  const session = createSession();
  const superLongWorkflow = {
    index: 102,
    name: "super-long",
    isSuperLong: true,
    deepNodeCount: 12,
    graph,
  };
  const runner = loadRunner(clock, session);
  const result = await runner.runSuperLong(
    session,
    superLongWorkflow,
    "disposable-workspace",
    {
      now: () => clock.value,
      sleep: () => {
        clock.value += clock.step;
      },
    },
  );
  assert.equal(result.status, "succeeded");
  assert.equal(result.meetsDuration, true);
  assert.equal(result.meetsNodeCount, true);
  assert.equal(result.meetsDeepNodes, true);
  assert.equal(result.qualified, true);
  assert.equal(result.deepNodeCount, 12);

  const quick = { ...superLongWorkflow, deepNodeCount: 3 };
  const short = await runner.runSuperLong(
    session,
    quick,
    "disposable-workspace",
    {
      now: () => 0,
      sleep: () => {},
    },
  );
  assert.equal(short.meetsDeepNodes, false);
  assert.equal(short.qualified, false);
});

test("missing endurance evidence and failed scenarios cannot produce a release certification", () => {
  const source = fs
    .readFileSync(new URL("report.mjs", directory), "utf8")
    .replace(/^export /gm, "");
  const report = vm.runInNewContext(
    `${source}\nbuildQualificationReport(input);`,
    {
      input: {
        context: {
          workspaceId: "disposable-workspace",
          build: { version: "test-build" },
        },
        workflows: [workflow],
        analyses: [{ index: 101, valid: false, error: "Invalid graph" }],
        results: [],
      },
    },
  );
  assert.doesNotMatch(report, /QUALIFIED|准予发布|全部指标合格/);
  assert.doesNotMatch(report, /60\.00/);
  assert.match(report, /NOT MEASURED|未测量/);
});

test("a failed hour-long endurance run cannot be hidden by another successful round", async () => {
  const clock = { value: 0, step: 3_600_000 };
  const session = createSession("failed");
  const result = await loadRunner(clock, session).runEnduranceStress(
    session,
    workflow,
    "disposable-workspace",
    {
      now: () => clock.value,
      sleep: () => {
        clock.value += clock.step;
      },
    },
  );
  assert.equal(result.qualified, false);
  assert.equal(result.status, "failed");
  assert.equal(
    session.calls.filter((call) => call.command === "create_workflow_run")
      .length,
    1,
  );
});

test("a short successful execution does not count as one hour of endurance", async () => {
  const clock = { value: 0 };
  const session = createSession();
  const invoke = session.invoke.bind(session);
  session.invoke = async (command, request) => {
    const response = await invoke(command, request);
    if (command === "get_workflow_run") response.run.finishedAt = 500;
    return response;
  };
  const result = await loadRunner(clock, session).runEnduranceStress(
    session,
    workflow,
    "disposable-workspace",
    {
      now: () => clock.value,
      sleep: (milliseconds) => {
        clock.value += milliseconds;
      },
    },
  );
  assert.equal(result.executionDurationMs, 500);
  assert.equal(result.qualified, false);
  assert.equal(
    session.calls.filter((call) => call.command === "create_workflow_run")
      .length,
    1,
  );
});

test("a cancellation that never settles retains evidence and halts further cleanup", async () => {
  const clock = { value: 0 };
  const session = createSession("running");
  const invoke = session.invoke.bind(session);
  session.invoke = (command, request) => {
    if (command === "cancel_workflow_run") {
      session.calls.push({ command, request });
      return {};
    }
    return invoke(command, request);
  };
  const result = await loadRunner(clock, session).runScenario(
    session,
    workflow,
    "disposable-workspace",
    {
      timeoutMs: 30,
      cancelTimeoutMs: 20,
      pollIntervalMs: 10,
      now: () => clock.value,
      sleep: (milliseconds) => {
        clock.value += milliseconds;
      },
    },
  );
  assert.equal(result.cleanupComplete, false);
  assert.match(result.cleanupErrors[0], /Cancellation did not settle/);
  assert.ok(session.calls.every((call) => !call.command.startsWith("delete_")));
});

test("interruption during polling cancels the run and removes only owned resources", async () => {
  const clock = { value: 0 };
  const session = createSession("running");
  const controller = new AbortController();
  const result = await loadRunner(clock, session).runScenario(
    session,
    workflow,
    "disposable-workspace",
    {
      now: () => clock.value,
      sleep: (milliseconds) => {
        clock.value += milliseconds;
        controller.abort();
      },
      signal: controller.signal,
    },
  );
  assert.equal(result.status, "error");
  assert.match(result.error, /interrupted/);
  assert.equal(result.cleanupComplete, true);
  const lifecycle = session.calls.filter((call) =>
    ["cancel_workflow_run", "delete_workflow_run", "delete_workflow"].includes(
      call.command,
    ),
  );
  assert.equal(
    JSON.stringify(lifecycle),
    JSON.stringify([
      { command: "cancel_workflow_run", request: { runId: "run-1" } },
      { command: "delete_workflow_run", request: { runId: "run-1" } },
      { command: "delete_workflow", request: { workflowId: "workflow-1" } },
    ]),
  );
});

test("a late successful response cannot turn an exceeded run deadline into a pass", async () => {
  const clock = { value: 0 };
  const session = createSession();
  const invoke = session.invoke.bind(session);
  session.invoke = (command, request) => {
    if (command === "get_workflow_run") clock.value += 100;
    return invoke(command, request);
  };
  const result = await loadRunner(clock, session).runScenario(
    session,
    workflow,
    "disposable-workspace",
    {
      timeoutMs: 50,
      pollIntervalMs: 10,
      now: () => clock.value,
      sleep: (milliseconds) => {
        clock.value += milliseconds;
      },
    },
  );
  assert.equal(result.status, "timeout");
  assert.equal(result.qualified, false);
});
