import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  createMockWorkflow as createMockWorkflowFixture,
  createParallelMockWorkflow as createParallelMockWorkflowFixture,
  createStaggeredParallelMockWorkflow as createStaggeredParallelMockWorkflowFixture,
} from "@ora/workflow-mock";
import {
  normalizeWorkflowDefinition,
  type GraphWorkflowRun,
  type WorkflowDefinition,
  type WorkflowRuntime,
} from "../src/index";
import {
  createMemoryWorkflowRuntime,
  executionOrder,
  planMockExecution,
} from "../src/memory";

/** Produces the transport-neutral definition used by runtime tests. */
function createMockWorkflow(locale: "zh-CN" | "en-US"): WorkflowDefinition {
  return normalizeWorkflowDefinition(createMockWorkflowFixture(locale));
}

/** Produces the normalized parallel fixture used by runtime tests. */
function createParallelMockWorkflow(locale: "zh-CN" | "en-US"): WorkflowDefinition {
  return normalizeWorkflowDefinition(createParallelMockWorkflowFixture(locale));
}

/** Produces the normalized staggered fixture used by runtime tests. */
function createStaggeredParallelMockWorkflow(
  locale: "zh-CN" | "en-US",
): WorkflowDefinition {
  return normalizeWorkflowDefinition(createStaggeredParallelMockWorkflowFixture(locale));
}

/** Builds a valid payload for whatever fields the open gate requires. */
function hitlPayloadFor(
  fields: { name: string; type: string; options?: { value: string }[] }[],
): Record<string, string> {
  const payload: Record<string, string> = {};
  for (const field of fields) {
    if (field.type === "select") {
      payload[field.name] = field.options?.[0]?.value ?? "diff";
    } else {
      payload[field.name] = "looks good";
    }
  }
  return payload;
}

/** Default mixed-schema payload (scope + notes). */
const HITL_PAYLOAD = { notes: "looks good", scope: "diff" };

function isTerminalRun(status: GraphWorkflowRun["status"]): boolean {
  return (
    status === "succeeded"
    || status === "failed"
    || status === "cancelled"
  );
}

/**
 * Advances timers and auto-submits open HITL gates until the run ends
 * or `maxWaves` is exhausted.
 */
async function drainRun(
  runtime: WorkflowRuntime,
  runId: string,
  stepMs: number,
  maxWaves = 40,
): Promise<GraphWorkflowRun | null> {
  for (let i = 0; i < maxWaves; i += 1) {
    const run = await runtime.runs.get(runId);
    if (run === null || isTerminalRun(run.status)) {
      return run;
    }
    if (run.openHitls.length > 0) {
      const gate = run.openHitls[0]!;
      await runtime.runs.submitHitl(run.id, gate.id, hitlPayloadFor(gate.schema.fields));
      continue;
    }
    await vi.advanceTimersByTimeAsync(stepMs);
  }
  return runtime.runs.get(runId);
}

describe("createMemoryWorkflowRuntime", () => {
  it("does not lose synchronous events during cursor replay handoff", async () => {
    const runtime = createMemoryWorkflowRuntime({
      autoStart: false,
      nodeStepMs: 60_000,
    });
    const definition = createMockWorkflow("en-US");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    await runtime.runs.start(run.id);

    const observed: string[] = [];
    const unsubscribe = runtime.runs.subscribe(
      run.id,
      (event) => {
        observed.push(event.type);
        if (event.type === "run_started") {
          void runtime.runs.cancel(run.id);
        }
      },
      { afterCursor: null },
    );

    expect(observed.at(-1)).toBe("run_finished");
    expect(observed.filter((type) => type === "run_finished")).toHaveLength(1);
    unsubscribe();
    runtime.dispose();
  });

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
    const renamed = await runtime.runs.rename(run.id, "  ????  ");
    expect(renamed).toEqual(
      expect.objectContaining({
        id: run.id,
        name: "????",
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
        description: "?????",
        instruction: "?????",
      },
    );
    const patchedNode = patched.definitionSnapshot.nodes.find(
      (node) => node.id === startNode!.id,
    );
    expect(patchedNode?.data).toEqual(
      expect.objectContaining({
        description: "?????",
        instruction: "?????",
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

  it("replays events created after an atomic live snapshot cursor", async () => {
    const runtime = createMemoryWorkflowRuntime({ autoStart: false });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    const snapshot = await runtime.runs.getLiveSnapshot(run.id);
    expect(snapshot).toEqual(expect.objectContaining({ cursor: null, artifacts: [] }));

    await runtime.runs.start(run.id);
    const replayed: Array<{ cursor: string; sequence: number; type: string }> = [];
    runtime.runs.subscribe(
      run.id,
      (event) => replayed.push({
        cursor: event.cursor,
        sequence: event.sequence,
        type: event.type,
      }),
      { afterCursor: snapshot!.cursor },
    );

    expect(replayed).toEqual([
      { cursor: `${run.id}:1`, sequence: 1, type: "run_started" },
      { cursor: `${run.id}:2`, sequence: 2, type: "node_started" },
      { cursor: `${run.id}:3`, sequence: 3, type: "node_conversation_item_upserted" },
    ]);
    runtime.dispose();
  });

  it("isolates a failing listener from the engine and sibling observers", async () => {
    const listenerErrors: unknown[] = [];
    const runtime = createMemoryWorkflowRuntime({
      autoStart: false,
      onListenerError: (error) => listenerErrors.push(error),
    });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    const observed: string[] = [];
    runtime.runs.subscribe(run.id, () => {
      throw new Error("observer failed");
    });
    runtime.runs.subscribe(run.id, (event) => observed.push(event.type));

    await runtime.runs.start(run.id);

    expect(listenerErrors).toHaveLength(3);
    expect(observed).toEqual([
      "run_started",
      "node_started",
      "node_conversation_item_upserted",
    ]);
    expect((await runtime.runs.get(run.id))?.status).toBe("running");
    runtime.dispose();
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
    const finished = await drainRun(runtime, run.id, 100);
    unsubscribe();

    expect(finished?.status).toBe("succeeded");
    expect(finished?.totals.tokenUsage?.totalTokens).toBeGreaterThan(0);
    expect(events[0]).toBe("run_started");
    expect(events.at(-1)).toBe("run_finished");
    for (const nodeId of plan.order) {
      expect(events).toContain(`node_started:${nodeId}:`);
      expect(events).toContain(`node_finished:${nodeId}:succeeded`);
    }
    // Default zh path takes??????so tests stays reachable ? nothing skipped.
    expect(plan.skipped).toEqual([]);
    expect(finished?.nodeStates.start?.tokenUsage).toBeUndefined();
    expect(finished?.nodeStates.understand?.tokenUsage?.totalTokens).toBeGreaterThan(0);
    const artifacts = await runtime.runs.listArtifacts(run.id);
    expect(artifacts.length).toBeGreaterThan(0);
    expect(artifacts.every((item) => item.nodeId.length > 0)).toBe(true);
    expect(artifacts.some((item) => item.kind === "markdown")).toBe(true);
    const liveSnapshot = await runtime.runs.getLiveSnapshot(run.id);
    expect(liveSnapshot?.conversation).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          nodeId: "review",
          sessionId: `workflow-node:${run.id}:review`,
          kind: "message",
          role: "assistant",
          status: "complete",
          markdown: expect.any(String),
        }),
        expect.objectContaining({
          nodeId: "review",
          sessionId: `workflow-node:${run.id}:review`,
          kind: "message",
          role: "user",
          markdown: expect.any(String),
        }),
        expect.objectContaining({
          nodeId: "review",
          sessionId: `workflow-node:${run.id}:review`,
          kind: "activity",
          activityKind: "thought",
        }),
      ]),
    );
    expect(finished?.nodeStates.review?.sessionId).toBe(
      `workflow-node:${run.id}:review`,
    );
  });

  it("pauses on prompt nodes until HITL is submitted", async () => {
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
    const events: string[] = [];
    runtime.runs.subscribe(run.id, (event) => {
      events.push(event.type);
    });

    await runtime.runs.start(run.id);
    await vi.advanceTimersByTimeAsync(100);

    const paused = await runtime.runs.get(run.id);
    expect(paused?.status).toBe("awaiting_input");
    expect(paused?.nodeStates.understand?.status).toBe("awaiting_input");
    expect(paused?.openHitls).toEqual([
      expect.objectContaining({
        nodeId: "understand",
        status: "open",
        policy: "wait",
        blocking: true,
        createdAt: expect.any(String),
        schema: expect.objectContaining({
          kind: "clarify",
          prompt: expect.any(String),
        }),
      }),
    ]);
    expect(events).toContain("hitl_required");

    await runtime.runs.submitHitl(
      run.id,
      paused!.openHitls[0]!.id,
      hitlPayloadFor(paused!.openHitls[0]!.schema.fields),
    );
    expect(events).toContain("hitl_resolved");
    expect(await runtime.runs.get(run.id)).toEqual(
      expect.objectContaining({
        status: "running",
        openHitls: [],
      }),
    );
    expect(
      (await runtime.runs.get(run.id))?.nodeStates.understand,
    ).toEqual(
      expect.objectContaining({
        status: "succeeded",
        input: expect.objectContaining({ summary: expect.any(String) }),
        output: expect.objectContaining({ summary: expect.any(String) }),
      }),
    );
    expect((await runtime.runs.getLiveSnapshot(run.id))?.conversation).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          nodeId: "understand",
          kind: "message",
          role: "assistant",
        }),
        expect.objectContaining({
          nodeId: "understand",
          kind: "message",
          role: "user",
          markdown: "looks good",
        }),
      ]),
    );

    const finished = await drainRun(runtime, run.id, 100);
    expect(finished?.status).toBe("succeeded");
  });

  it("appends the approval answer and an ack into the node conversation", async () => {
    vi.useFakeTimers();
    const runtime = createMemoryWorkflowRuntime({
      nodeStepMs: 100,
      autoStart: false,
      locale: "zh-CN",
    });
    const definition = createStaggeredParallelMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    await runtime.runs.start(run.id);
    // quick_scan is a short approval gate on the staggered fixture.
    await vi.advanceTimersByTimeAsync(1_000);
    const paused = await runtime.runs.get(run.id);
    const gate = paused?.openHitls.find((item) => item.nodeId === "quick_scan");
    expect(gate).toEqual(expect.objectContaining({
      nodeId: "quick_scan",
      schema: expect.objectContaining({ kind: "approval" }),
    }));
    await runtime.runs.submitHitl(
      run.id,
      gate!.id,
      hitlPayloadFor(gate!.schema.fields),
    );
    const conversation = (await runtime.runs.getLiveSnapshot(run.id))?.conversation
      ?? [];
    const nodeItems = conversation.filter((item) => item.nodeId === "quick_scan");
    expect(nodeItems.some((item) => item.kind === "message" && item.role === "user")).toBe(
      true,
    );
    expect(
      nodeItems.some((item) =>
        item.kind === "message"
        && item.role === "assistant"
        && item.markdown.includes("已收到你的确认")
      ),
    ).toBe(true);
    runtime.dispose();
    vi.useRealTimers();
  });

  it("rejects submitHitl without an open request or required fields", async () => {
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
    await expect(
      runtime.runs.submitHitl(run.id, "missing", HITL_PAYLOAD),
    ).rejects.toThrow(/no open hitl/i);

    await runtime.runs.start(run.id);
    await vi.advanceTimersByTimeAsync(100);
    const paused = await runtime.runs.get(run.id);
    await expect(
      runtime.runs.submitHitl(run.id, "wrong-id", HITL_PAYLOAD),
    ).rejects.toThrow(/no open hitl request/i);
    await expect(
      runtime.runs.submitHitl(run.id, paused!.openHitls[0]!.id, { scope: "diff" }),
    ).rejects.toThrow(/missing required field answer/i);
  });

  it("rejects invalid select options on HITL submit", async () => {
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
    await vi.advanceTimersByTimeAsync(100);
    const paused = await runtime.runs.get(run.id);
    const gate = paused!.openHitls[0]!;
    await expect(
      runtime.runs.submitHitl(run.id, gate.id, { scope: "not-a-real-option" }),
    ).rejects.toThrow(/invalid option/i);
  });

  it("keeps waiting for HITL without timing out", async () => {
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
    await runtime.runs.start(run.id);
    await vi.advanceTimersByTimeAsync(100);
    expect((await runtime.runs.get(run.id))?.status).toBe("awaiting_input");

    await vi.advanceTimersByTimeAsync(60_000);
    const stillWaiting = await runtime.runs.get(run.id);
    expect(stillWaiting?.status).toBe("awaiting_input");
    expect(stillWaiting?.openHitls).toEqual([
      expect.objectContaining({ status: "open" }),
    ]);
  });

  it("clears HITL when the run is cancelled", async () => {
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
    await runtime.runs.start(run.id);
    await vi.advanceTimersByTimeAsync(100);
    expect((await runtime.runs.get(run.id))?.openHitls).toEqual([
      expect.objectContaining({ status: "open" }),
    ]);

    await runtime.runs.cancel(run.id);
    expect(await runtime.runs.get(run.id)).toEqual(
      expect.objectContaining({
        status: "cancelled",
        openHitls: [],
      }),
    );
  });

  it("leaves the validation branch unexecuted when kickoff prefers documentation", async () => {
    const runtime = createMemoryWorkflowRuntime({
      nodeStepMs: 50,
      autoStart: false,
    });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
      kickoffInput: "update README docs only",
    });
    await runtime.runs.start(run.id);
    const plan = planMockExecution(definition, {
      kickoffInput: "update README docs only",
    });
    expect(plan.skipped).toContain("tests");
    expect((await runtime.runs.get(run.id))?.nodeStates.tests?.status).toBe("idle");

    const finished = await drainRun(runtime, run.id, 50);
    expect(finished?.status).toBe("succeeded");
    expect(finished?.nodeStates.tests?.status).toBe("idle");
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

    // start completes ? gather HITL
    await vi.advanceTimersByTimeAsync(100);
    const paused = await runtime.runs.get(run.id);
    expect(paused?.status).toBe("awaiting_input");
    expect(paused?.nodeStates.gather?.status).toBe("awaiting_input");
    await runtime.runs.submitHitl(
      run.id,
      paused!.openHitls[0]!.id,
      hitlPayloadFor(paused!.openHitls[0]!.schema.fields),
    );

    const mid = await runtime.runs.get(run.id);
    expect(mid?.nodeStates.security?.status).toBe("running");
    expect(mid?.nodeStates.quality?.status).toBe("running");
    // docs is also a prompt node ? second concurrent HITL gate in the wave
    expect(mid?.nodeStates.docs?.status).toBe("awaiting_input");
    expect(mid?.openHitls.map((item) => item.nodeId)).toEqual(["docs"]);
    expect(mid?.status).toBe("awaiting_input");

    const finished = await drainRun(runtime, run.id, 100);
    expect(finished).toEqual(
      expect.objectContaining({ status: "succeeded" }),
    );
  });

  it("staggers parallel starts and ends via per-node mockStepMs", async () => {
    const runtime = createMemoryWorkflowRuntime({
      // Default would not apply ? every fixture node sets mockStepMs.
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
    // quick_scan is prompt ? HITL; lint/index start on timers.
    expect(snap?.nodeStates).toEqual(
      expect.objectContaining({
        start: expect.objectContaining({ status: "succeeded", durationMs: 800 }),
        quick_scan: expect.objectContaining({ status: "awaiting_input" }),
        lint: expect.objectContaining({ status: "running" }),
        slow_index: expect.objectContaining({ status: "running" }),
        deep_security: expect.objectContaining({ status: "idle" }),
        docs_pass: expect.objectContaining({ status: "idle" }),
      }),
    );
    expect(snap?.openHitls.map((item) => item.nodeId)).toEqual(["quick_scan"]);

    // Hold the open HITL while timers advance ? mirrors a slow human response.
    // lint ends at 3500 from its start (after the 800ms start wave)
    await vi.advanceTimersByTimeAsync(3_500);
    snap = await runtime.runs.get(run.id);
    expect(snap?.nodeStates.lint).toEqual(
      expect.objectContaining({ status: "succeeded", durationMs: 3_500 }),
    );
    expect(snap?.nodeStates.slow_index?.status).toBe("running");
    expect(snap?.nodeStates.deep_security?.status).toBe("idle");
    expect(snap?.openHitls.map((item) => item.nodeId)).toEqual(["quick_scan"]);

    // slow_index ends at 5500; docs_pass opens its own concurrent HITL gate.
    await vi.advanceTimersByTimeAsync(2_000);
    snap = await runtime.runs.get(run.id);
    expect(snap?.nodeStates.slow_index).toEqual(
      expect.objectContaining({ status: "succeeded", durationMs: 5_500 }),
    );
    expect(snap?.nodeStates.docs_pass?.status).toBe("awaiting_input");
    expect(snap?.openHitls.map((item) => item.nodeId).sort()).toEqual([
      "docs_pass",
      "quick_scan",
    ]);
    expect(snap?.nodeStates.deep_security?.status).toBe("idle");
    expect(snap?.nodeStates.join?.status).toBe("idle");

    // User can resolve docs_pass first while quick_scan is still open.
    const docsGate = snap!.openHitls.find((item) => item.nodeId === "docs_pass");
    expect(docsGate).toBeDefined();
    await runtime.runs.submitHitl(
      run.id,
      docsGate!.id,
      hitlPayloadFor(docsGate!.schema.fields),
    );
    snap = await runtime.runs.get(run.id);
    expect(snap?.nodeStates.docs_pass?.status).toBe("succeeded");
    expect(snap?.openHitls.map((item) => item.nodeId)).toEqual(["quick_scan"]);
    expect(snap?.status).toBe("awaiting_input");

    await runtime.runs.submitHitl(
      run.id,
      snap!.openHitls[0]!.id,
      hitlPayloadFor(snap!.openHitls[0]!.schema.fields),
    );
    snap = await runtime.runs.get(run.id);
    expect(snap?.nodeStates.quick_scan?.status).toBe("succeeded");
    expect(snap?.nodeStates.deep_security?.status).toBe("running");
    expect(snap?.openHitls).toEqual([]);

    // Drain deep_security remainder + join + output
    const finished = await drainRun(runtime, run.id, 1_000);
    expect(finished).toEqual(
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
    const finished = await drainRun(runtime, second.id, 100);
    expect(await runtime.runs.get(first.id)).toEqual(
      expect.objectContaining({ status: "cancelled" }),
    );
    expect(finished).toEqual(
      expect.objectContaining({ status: "succeeded" }),
    );
  });

  /**
   * Demo-path smoke: mount -> start -> drain HITL gates -> succeeded.
   * Mirrors the manual Theater checklist without browser e2e.
   */
  it("demo path: mount, start, drain HITL, succeed", async () => {
    const stepMs = 50;
    const runtime = createMemoryWorkflowRuntime({
      nodeStepMs: stepMs,
      autoStart: false,
    });
    const definition = createMockWorkflow("zh-CN");
    await runtime.host.mount("p1", definition);
    const created = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    expect(created.status).toBe("pending");

    const types: string[] = [];
    runtime.runs.subscribe(created.id, (event) => {
      types.push(event.type);
    });

    await runtime.runs.start(created.id);
    expect(types).toContain("run_started");

    const finished = await drainRun(runtime, created.id, stepMs);
    expect(finished).toEqual(
      expect.objectContaining({
        status: "succeeded",
        openHitls: [],
      }),
    );
    expect(types).toContain("hitl_required");
    expect(types).toContain("hitl_resolved");
    expect(types.at(-1)).toBe("run_finished");
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
    const plan = planMockExecution(workflow, { kickoffInput: "update README docs only" });
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
