import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createMockWorkflow,
  createParallelMockWorkflow,
  createStaggeredParallelMockWorkflow,
} from "@ora/workflow-mock";
import { createMemoryWorkflowRuntime } from "./memory-workflow-runtime";
import { planMockExecution } from "./mock-execution-plan";
import { executionOrder } from "./mock-run-engine";

describe("createMemoryWorkflowRuntime", () => {
  it("mounts the same definition on multiple projects by reference", async () => {
    const runtime = createMemoryWorkflowRuntime({ autoStart: false });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    await runtime.host.mount("p2", definition);
    expect(await runtime.host.listMounts("p1")).toEqual([
      expect.objectContaining({ projectId: "p1", definitionId: definition.id }),
    ]);
    expect(await runtime.host.listMounts("p2")).toEqual([
      expect.objectContaining({ projectId: "p2", definitionId: definition.id }),
    ]);
  });

  it("freezes a definition snapshot when creating a run", async () => {
    const runtime = createMemoryWorkflowRuntime({ autoStart: false });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
      kickoffInput: "review main",
    });
    definition.name = "mutated-library-name";
    const stored = await runtime.runs.get(run.id);
    expect(stored).toEqual(
      expect.objectContaining({
        id: run.id,
        kickoffInput: "review main",
        status: "pending",
        name: run.name,
      }),
    );
    expect(stored?.definitionSnapshot.name).toBe(run.name);
    expect(stored?.definitionSnapshot.name).not.toBe("mutated-library-name");
  });

  it("rejects create when the definition is not mounted on the project", async () => {
    const runtime = createMemoryWorkflowRuntime({ autoStart: false });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    await expect(
      runtime.runs.create({ projectId: "p2", definitionId: definition.id }),
    ).rejects.toThrow(/not mounted/);
  });

  it("cancels an open run and emits run_finished", async () => {
    const runtime = createMemoryWorkflowRuntime({ autoStart: false });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    const events: string[] = [];
    const unsubscribe = runtime.runs.subscribe(run.id, (event) => {
      events.push(event.type);
    });
    await runtime.runs.cancel(run.id);
    unsubscribe();
    expect(events).toEqual(["run_finished"]);
    expect(await runtime.runs.get(run.id)).toEqual(
      expect.objectContaining({ status: "cancelled" }),
    );
  });

  it("upserts a single mount but allows multiple runs on the same project", async () => {
    const runtime = createMemoryWorkflowRuntime({ autoStart: false });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    definition.description = "updated blob";
    await runtime.host.mount("p1", definition);
    expect(await runtime.host.listMounts("p1")).toHaveLength(1);
    expect(await runtime.host.listMountsByDefinition(definition.id)).toEqual([
      expect.objectContaining({ projectId: "p1", definitionId: definition.id }),
    ]);
    const first = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    const second = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    expect(first.id).not.toBe(second.id);
    expect(await runtime.runs.list("p1")).toHaveLength(2);
    expect(second.definitionSnapshot.description).toBe("updated blob");
  });

  it("deletes one run without affecting a sibling run", async () => {
    const runtime = createMemoryWorkflowRuntime({ autoStart: false });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const first = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    const second = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    await runtime.runs.delete(first.id);
    expect(await runtime.runs.get(first.id)).toBeNull();
    expect(await runtime.runs.get(second.id)).toEqual(
      expect.objectContaining({ id: second.id }),
    );
  });

  it("renames a run without changing its definition snapshot", async () => {
    const runtime = createMemoryWorkflowRuntime({ autoStart: false });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    const renamed = await runtime.runs.rename(run.id, "  审查一轮  ");
    expect(renamed).toEqual(
      expect.objectContaining({
        id: run.id,
        name: "审查一轮",
        definitionSnapshot: run.definitionSnapshot,
      }),
    );
  });

  it("patches pending snapshot node copy without touching the library definition", async () => {
    const runtime = createMemoryWorkflowRuntime({ autoStart: false });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    const startNode = run.definitionSnapshot.nodes.find(
      (node) => node.data.kind === "start",
    );
    expect(startNode).toBeDefined();

    const patched = await runtime.runs.updateSnapshotNode(
      run.id,
      startNode!.id,
      {
        description: "仅本次说明",
        instruction: "仅本次指令",
      },
    );
    const patchedNode = patched.definitionSnapshot.nodes.find(
      (node) => node.id === startNode!.id,
    );
    expect(patchedNode?.data).toEqual(
      expect.objectContaining({
        description: "仅本次说明",
        instruction: "仅本次指令",
      }),
    );

    const library = await runtime.host.getDefinition(definition.id);
    const libraryNode = library?.nodes.find((node) => node.id === startNode!.id);
    expect(libraryNode?.data.description).toBe(startNode!.data.description);
    expect(libraryNode?.data.instruction).toBe(startNode!.data.instruction);
  });

  it("rejects snapshot node edits once the run is no longer pending", async () => {
    const runtime = createMemoryWorkflowRuntime({
      autoStart: false,
      nodeStepMs: 100,
    });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    const nodeId = run.definitionSnapshot.nodes[0]!.id;
    await runtime.runs.start(run.id);
    await expect(
      runtime.runs.updateSnapshotNode(run.id, nodeId, {
        description: "too late",
      }),
    ).rejects.toThrow(/pending/i);
  });

  it("rejects snapshot edits for unknown nodes", async () => {
    const runtime = createMemoryWorkflowRuntime({ autoStart: false });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    await expect(
      runtime.runs.updateSnapshotNode(run.id, "missing-node", {
        instruction: "x",
      }),
    ).rejects.toThrow(/unknown snapshot node/i);
  });
});

describe("mock run engine", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("emits ordered run/node events through completion on the default path", async () => {
    const runtime = createMemoryWorkflowRuntime({
      nodeStepMs: 100,
      autoStart: false,
    });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    expect(run.status).toBe("pending");

    const events: string[] = [];
    const unsubscribe = runtime.runs.subscribe(run.id, (event) => {
      if (event.type === "node_started" || event.type === "node_finished") {
        events.push(`${event.type}:${event.nodeId}:${event.type === "node_finished" ? event.status : ""}`);
      } else {
        events.push(event.type);
      }
    });

    await runtime.runs.start(run.id);
    expect(await runtime.runs.get(run.id)).toEqual(
      expect.objectContaining({ status: "running" }),
    );
    expect(events[0]).toBe("run_started");

    const plan = planMockExecution(definition, {});
    for (let i = 0; i < plan.order.length; i += 1) {
      await vi.advanceTimersByTimeAsync(100);
    }
    unsubscribe();

    const finished = await runtime.runs.get(run.id);
    expect(finished?.status).toBe("succeeded");
    expect(finished?.totals.tokenUsage?.totalTokens).toBeGreaterThan(0);
    expect(events[0]).toBe("run_started");
    expect(events.at(-1)).toBe("run_finished");
    for (const nodeId of plan.order) {
      expect(events).toContain(`node_started:${nodeId}:`);
      expect(events).toContain(`node_finished:${nodeId}:succeeded`);
    }
    // Default zh path takes「需要检查」so tests stays reachable — nothing skipped.
    expect(plan.skipped).toEqual([]);
    expect(finished?.nodeStates.start?.tokenUsage).toBeUndefined();
    expect(finished?.nodeStates.understand?.tokenUsage?.totalTokens).toBeGreaterThan(0);
    const artifacts = await runtime.runs.listArtifacts(run.id);
    expect(artifacts.length).toBeGreaterThan(0);
    expect(artifacts.every((item) => item.nodeId.length > 0)).toBe(true);
    expect(artifacts.some((item) => item.kind === "markdown")).toBe(true);
  });

  it("skips the validation branch when kickoff prefers documentation", async () => {
    const runtime = createMemoryWorkflowRuntime({
      nodeStepMs: 50,
      autoStart: false,
    });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
      kickoffInput: "只更新 README 文档说明",
    });
    await runtime.runs.start(run.id);
    const plan = planMockExecution(definition, {
      kickoffInput: "只更新 README 文档说明",
    });
    expect(plan.skipped).toContain("tests");
    expect((await runtime.runs.get(run.id))?.nodeStates.tests?.status).toBe("skipped");

    for (let i = 0; i < plan.order.length; i += 1) {
      await vi.advanceTimersByTimeAsync(50);
    }
    const finished = await runtime.runs.get(run.id);
    expect(finished?.status).toBe("succeeded");
    expect(finished?.nodeStates.tests?.status).toBe("skipped");
    expect(finished?.nodeStates.output?.status).toBe("succeeded");
  });

  it("runs independent fan-out branches in parallel", async () => {
    const runtime = createMemoryWorkflowRuntime({
      nodeStepMs: 100,
      autoStart: false,
    });
    const definition = createParallelMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    await runtime.runs.start(run.id);

    // start → gather (two sequential waves)
    await vi.advanceTimersByTimeAsync(100);
    await vi.advanceTimersByTimeAsync(100);

    const mid = await runtime.runs.get(run.id);
    expect(mid?.nodeStates.security?.status).toBe("running");
    expect(mid?.nodeStates.quality?.status).toBe("running");
    expect(mid?.nodeStates.docs?.status).toBe("running");

    // Drain remaining waves (parallel trio + synthesize + output)
    for (let i = 0; i < 4; i += 1) {
      await vi.advanceTimersByTimeAsync(100);
    }
    expect(await runtime.runs.get(run.id)).toEqual(
      expect.objectContaining({ status: "succeeded" }),
    );
  });

  it("staggers parallel starts and ends via per-node mockStepMs", async () => {
    const runtime = createMemoryWorkflowRuntime({
      // Default would not apply — every fixture node sets mockStepMs.
      nodeStepMs: 50_000,
      autoStart: false,
    });
    const definition = createStaggeredParallelMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    await runtime.runs.start(run.id);

    await vi.advanceTimersByTimeAsync(800);
    let snap = await runtime.runs.get(run.id);
    expect(snap?.nodeStates).toEqual(
      expect.objectContaining({
        start: expect.objectContaining({ status: "succeeded", durationMs: 800 }),
        quick_scan: expect.objectContaining({ status: "running" }),
        lint: expect.objectContaining({ status: "running" }),
        slow_index: expect.objectContaining({ status: "running" }),
        deep_security: expect.objectContaining({ status: "idle" }),
        docs_pass: expect.objectContaining({ status: "idle" }),
      }),
    );

    // quick_scan finishes first → deep_security starts while lint/index still run
    await vi.advanceTimersByTimeAsync(1_500);
    snap = await runtime.runs.get(run.id);
    expect(snap?.nodeStates.quick_scan).toEqual(
      expect.objectContaining({ status: "succeeded", durationMs: 1_500 }),
    );
    expect(snap?.nodeStates.deep_security?.status).toBe("running");
    expect(snap?.nodeStates.lint?.status).toBe("running");
    expect(snap?.nodeStates.slow_index?.status).toBe("running");
    expect(snap?.nodeStates.docs_pass?.status).toBe("idle");

    // lint ends; deep_security + slow_index still overlap
    await vi.advanceTimersByTimeAsync(2_000);
    snap = await runtime.runs.get(run.id);
    expect(snap?.nodeStates.lint).toEqual(
      expect.objectContaining({ status: "succeeded", durationMs: 3_500 }),
    );
    expect(snap?.nodeStates.deep_security?.status).toBe("running");
    expect(snap?.nodeStates.slow_index?.status).toBe("running");
    expect(snap?.nodeStates.docs_pass?.status).toBe("idle");

    // slow_index ends → docs_pass starts late while deep_security still running
    await vi.advanceTimersByTimeAsync(2_000);
    snap = await runtime.runs.get(run.id);
    expect(snap?.nodeStates.slow_index).toEqual(
      expect.objectContaining({ status: "succeeded", durationMs: 5_500 }),
    );
    expect(snap?.nodeStates.docs_pass?.status).toBe("running");
    expect(snap?.nodeStates.deep_security?.status).toBe("running");
    expect(snap?.nodeStates.join?.status).toBe("idle");

    // Drain deep_security (2s left), docs_pass (2s), join, output
    await vi.advanceTimersByTimeAsync(2_000);
    await vi.advanceTimersByTimeAsync(2_000);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(await runtime.runs.get(run.id)).toEqual(
      expect.objectContaining({ status: "succeeded" }),
    );
  });

  it("ignores start() while a run is already running", async () => {
    const runtime = createMemoryWorkflowRuntime({
      nodeStepMs: 200,
      autoStart: false,
    });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    const events: string[] = [];
    runtime.runs.subscribe(run.id, (event) => {
      events.push(event.type);
    });
    await runtime.runs.start(run.id);
    await runtime.runs.start(run.id);
    expect(events.filter((type) => type === "run_started")).toHaveLength(1);
  });

  it("stops progression when cancelled mid-run", async () => {
    const runtime = createMemoryWorkflowRuntime({
      nodeStepMs: 200,
      autoStart: true,
    });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const types: string[] = [];
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    runtime.runs.subscribe(run.id, (event) => {
      types.push(event.type);
    });

    await vi.advanceTimersByTimeAsync(200);
    await runtime.runs.cancel(run.id);
    await vi.advanceTimersByTimeAsync(2000);

    expect(await runtime.runs.get(run.id)).toEqual(
      expect.objectContaining({ status: "cancelled" }),
    );
    expect(types.filter((type) => type === "run_finished")).toHaveLength(1);
    expect(types.at(-1)).toBe("run_finished");
    const finishedCount = types.filter((type) => type === "node_finished").length;
    expect(finishedCount).toBeLessThan(definition.nodes.length);
  });

  it("keeps concurrent runs independent when one is cancelled", async () => {
    const runtime = createMemoryWorkflowRuntime({
      nodeStepMs: 100,
      autoStart: true,
    });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const first = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    const second = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    await runtime.runs.cancel(first.id);
    const plan = planMockExecution(definition, {});
    for (let i = 0; i < plan.order.length; i += 1) {
      await vi.advanceTimersByTimeAsync(100);
    }
    expect(await runtime.runs.get(first.id)).toEqual(
      expect.objectContaining({ status: "cancelled" }),
    );
    expect(await runtime.runs.get(second.id)).toEqual(
      expect.objectContaining({ status: "succeeded" }),
    );
  });
});

describe("planMockExecution", () => {
  it("orders mock workflow nodes with start first on the default path", () => {
    const workflow = createMockWorkflow("zh-CN");
    const plan = planMockExecution(workflow);
    expect(plan.order[0]).toBe("start");
    expect(plan.order).toContain("output");
    expect(plan.order.indexOf("understand")).toBeLessThan(plan.order.indexOf("quality"));
    expect(plan.skipped).toEqual([]);
  });

  it("marks the unused exclusive branch as skipped for doc kickoff", () => {
    const workflow = createMockWorkflow("zh-CN");
    const plan = planMockExecution(workflow, { kickoffInput: "文档说明更新" });
    expect(plan.skipped).toEqual(["tests"]);
    expect(plan.order).not.toContain("tests");
    expect(plan.order).toContain("review");
  });
});

describe("executionOrder", () => {
  it("returns full-graph topo without applying exclusivity", () => {
    const workflow = createMockWorkflow("zh-CN");
    const order = executionOrder(workflow);
    expect(order).toHaveLength(workflow.nodes.length);
    expect(order[0]).toBe("start");
  });
});
