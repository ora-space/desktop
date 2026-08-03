import type { DemoWorkflow } from "@ora/workflow-mock";
import {
  createDefaultMockPathPolicy,
  nodeKindUsesTokens,
  planMockExecution,
  topologicalOrder,
  type MockExecutionPlan,
  type MockPathPolicy,
} from "./mock-execution-plan";
import type {
  GraphWorkflowNodeIo,
  GraphWorkflowNodeState,
  GraphWorkflowRun,
  GraphWorkflowTokenUsage,
  HitlRequest,
  HitlSchema,
  WorkflowArtifact,
  WorkflowRunEvent,
} from "./types";

export type MockHitlLocale = "zh-CN" | "en-US";

export interface MockRunEngineOptions {
  /** Duration of each node step. Default 5000ms so Theater switching is tryable. */
  nodeStepMs?: number;
  /** Condition path selection; defaults to kickoff-aware label heuristics. */
  pathPolicy?: MockPathPolicy;
  /** Locale for mock HITL schema copy. Default zh-CN. */
  locale?: MockHitlLocale;
}

export interface MockRunEngineHost {
  getRun: (runId: string) => GraphWorkflowRun | undefined;
  setRun: (run: GraphWorkflowRun) => void;
  appendArtifact: (artifact: WorkflowArtifact) => void;
  emit: (runId: string, event: WorkflowRunEvent) => void;
  notifyChanged: (run: GraphWorkflowRun) => void;
  nowIso: () => string;
  nextArtifactId: () => string;
  nextHitlId: () => string;
}

/** Truncates text for glanceable I/O summaries. */
function ioPreview(text: string, max = 96): string {
  const trimmed = text.trim().replace(/\s+/g, " ");
  if (trimmed.length <= max) {
    return trimmed;
  }
  return `${trimmed.slice(0, max - 1)}…`;
}

/** Builds mock HITL schema for a prompt node (approval / feedback / clarify). */
export function createMockHitlSchema(
  nodeId: string,
  locale: MockHitlLocale = "zh-CN",
): HitlSchema {
  const en = locale === "en-US";
  const scopeField = {
    name: "scope",
    type: "select" as const,
    label: en ? "Review scope" : "审查范围",
    required: true,
    options: [
      { value: "diff", label: en ? "Current changes only" : "仅当前改动" },
      { value: "branch", label: en ? "Whole branch" : "整个分支" },
    ],
  };
  const notesField = {
    name: "notes",
    type: "textarea" as const,
    label: en ? "Notes" : "补充说明",
    required: true,
    placeholder: en
      ? "e.g. focus on auth boundaries and edge cases"
      : "例如：重点关注权限与边界情况",
  };
  const answerField = {
    name: "answer",
    type: "textarea" as const,
    label: en ? "Your answer" : "你的回答",
    required: true,
    placeholder: en
      ? "Reply to the model’s question…"
      : "直接回复模型的问题…",
  };

  if (nodeId === "quick_scan" || nodeId === "docs") {
    return {
      kind: "approval",
      title: en ? "Confirmation needed" : "需要确认",
      prompt: en
        ? "Choose the review scope for this step before continuing."
        : "请选择本步审查范围后再继续。",
      fields: [scopeField],
    };
  }

  if (nodeId === "docs_pass") {
    return {
      kind: "feedback",
      title: en ? "Add feedback" : "补充反馈",
      prompt: en
        ? "After proofreading, note what later steps should watch for."
        : "校对完成后，请补充你希望后续步骤关注的点。",
      fields: [notesField],
    };
  }

  if (nodeId === "understand") {
    return {
      kind: "clarify",
      title: en ? "Clarification needed" : "需要你澄清",
      prompt: en
        ? "This change touches both the auth middleware and the route table. Should I prioritize permission boundaries, or map the route regression surface first?"
        : "这次改动同时动到了鉴权中间件和路由表。你希望我优先核对权限边界，还是先梳理路由回归范围？",
      fields: [answerField],
    };
  }

  return {
    kind: "feedback",
    title: en ? "Confirm understanding" : "确认本步理解",
    prompt: en
      ? "Confirm the reading, pick a scope, and add a short note."
      : "确认理解无误后选择范围，并补充说明。",
    fields: [scopeField, notesField],
  };
}

/** Stub input shown when a node starts. */
function stubNodeInput(
  run: GraphWorkflowRun,
  nodeId: string,
): GraphWorkflowNodeIo {
  const node = run.definitionSnapshot.nodes.find((item) => item.id === nodeId);
  const title = node?.data.title ?? nodeId;
  const kickoff = run.kickoffInput?.trim() ?? "";
  if (kickoff !== "") {
    return {
      summary: ioPreview(kickoff),
      detail: node?.data.instruction,
    };
  }
  return {
    summary: title,
    detail: node?.data.instruction,
  };
}

/** Stub output when a timed node finishes. */
function stubNodeOutput(
  run: GraphWorkflowRun,
  nodeId: string,
): GraphWorkflowNodeIo {
  const node = run.definitionSnapshot.nodes.find((item) => item.id === nodeId);
  const title = node?.data.title ?? nodeId;
  const kind = node?.data.kind ?? "agent";
  if (kind === "output") {
    return {
      summary: `Report: ${title}`,
      detail: node?.data.instruction,
    };
  }
  if (kind === "tool") {
    return {
      summary: `Tool finished: ${node?.data.tool ?? title}`,
      detail: node?.data.instruction,
    };
  }
  return {
    summary: `Completed: ${title}`,
    detail: node?.data.instruction,
  };
}

/** Summarizes a HITL submit payload for node output. */
function hitlAnswerOutput(
  schema: HitlSchema,
  payload: Record<string, unknown>,
): GraphWorkflowNodeIo {
  const parts: string[] = [];
  for (const field of schema.fields) {
    const raw = payload[field.name];
    if (raw === undefined || raw === null) {
      continue;
    }
    const text = String(raw).trim();
    if (text === "") {
      continue;
    }
    if (field.type === "select") {
      const label = field.options?.find((option) => option.value === text)?.label
        ?? text;
      parts.push(`${field.label}: ${label}`);
    } else {
      parts.push(`${field.label}: ${text}`);
    }
  }
  const joined = parts.join(" · ");
  return {
    summary: ioPreview(joined !== "" ? joined : "Submitted"),
    detail: joined !== "" ? joined : undefined,
  };
}

/**
 * Mock executor over a frozen DemoWorkflow snapshot.
 * Plans a reachable path (condition = exclusive), then runs ready nodes in
 * parallel waves: every node whose predecessors have succeeded starts together.
 * `prompt` nodes pause for HITL; other kinds use timed auto-complete.
 * Per-node `data.mockStepMs` overrides the default step duration so staggered
 * starts/ends can be demonstrated.
 */
export function createMockRunEngine(
  host: MockRunEngineHost,
  options: MockRunEngineOptions = {},
) {
  const nodeStepMs = options.nodeStepMs ?? 5_000;
  const pathPolicy = options.pathPolicy ?? createDefaultMockPathPolicy();
  const locale = options.locale ?? "zh-CN";
  /** Per-run map of nodeId → in-flight step timer. */
  const timers = new Map<string, Map<string, ReturnType<typeof setTimeout>>>();
  const plans = new Map<string, MockExecutionPlan>();

  /** Resolves step length: node mockStepMs when positive, else engine default. */
  function stepMsFor(run: GraphWorkflowRun, nodeId: string): number {
    const node = run.definitionSnapshot.nodes.find((item) => item.id === nodeId);
    const custom = node?.data.mockStepMs;
    if (typeof custom === "number" && Number.isFinite(custom) && custom > 0) {
      return custom;
    }
    return nodeStepMs;
  }

  /** Clears every pending step timer for a run (cancel / delete). */
  function stop(runId: string): void {
    const byNode = timers.get(runId);
    if (byNode !== undefined) {
      for (const timer of byNode.values()) {
        clearTimeout(timer);
      }
      timers.delete(runId);
    }
    plans.delete(runId);
  }

  function timersFor(runId: string): Map<string, ReturnType<typeof setTimeout>> {
    let byNode = timers.get(runId);
    if (byNode === undefined) {
      byNode = new Map();
      timers.set(runId, byNode);
    }
    return byNode;
  }

  /**
   * Starts every currently ready idle node. When nothing is left to run and no
   * timers remain, finishes the run as succeeded.
   */
  function pump(runId: string): void {
    const run = host.getRun(runId);
    const plan = plans.get(runId);
    if (run === undefined || plan === undefined || isTerminal(run.status)) {
      return;
    }

    const ready = plan.order.filter((nodeId) => {
      const state = run.nodeStates[nodeId];
      if (state === undefined || state.status !== "idle") {
        return false;
      }
      if (timersFor(runId).has(nodeId)) {
        return false;
      }
      const preds = plan.predecessors[nodeId] ?? [];
      return preds.every((predId) => {
        const pred = run.nodeStates[predId];
        return pred?.status === "succeeded" || pred?.status === "skipped";
      });
    });

    for (const nodeId of ready) {
      beginNode(runId, nodeId);
    }

    const latest = host.getRun(runId);
    if (latest === undefined || isTerminal(latest.status)) {
      return;
    }

    const allDone = plan.order.every((nodeId) => {
      const status = latest.nodeStates[nodeId]?.status;
      return (
        status === "succeeded"
        || status === "skipped"
        || status === "failed"
        || status === "cancelled"
      );
    });
    if (allDone && timersFor(runId).size === 0 && latest.openHitls.length === 0) {
      finishRun(runId, /*status*/ "succeeded");
    }
  }

  function beginNode(runId: string, nodeId: string): void {
    const run = host.getRun(runId);
    if (run === undefined || isTerminal(run.status)) {
      return;
    }
    if (run.nodeStates[nodeId]?.status !== "idle") {
      return;
    }
    const node = run.definitionSnapshot.nodes.find((item) => item.id === nodeId);
    if (node?.data.kind === "prompt") {
      beginHitl(runId, nodeId);
      return;
    }

    const startedAt = host.nowIso();
    const stepMs = stepMsFor(run, nodeId);
    const input = stubNodeInput(run, nodeId);
    patchNode(runId, nodeId, {
      status: "running",
      startedAt,
      input,
    });
    host.emit(runId, { type: "node_started", runId, nodeId });

    const timer = setTimeout(() => {
      timersFor(runId).delete(nodeId);
      const current = host.getRun(runId);
      if (current === undefined || current.status === "cancelled") {
        return;
      }
      completeNode(runId, nodeId, startedAt, stepMs);
      pump(runId);
    }, stepMs);
    timersFor(runId).set(nodeId, timer);
  }

  /**
   * Pauses a prompt node for human input until `submitHitl` (no timeout).
   * Multiple prompts may open gates concurrently; each gets its own request
   * in `openHitls` so the user can answer any of them.
   */
  function beginHitl(runId: string, nodeId: string): void {
    const run = host.getRun(runId);
    if (run === undefined || isTerminal(run.status)) {
      return;
    }
    if (run.nodeStates[nodeId]?.status !== "idle") {
      return;
    }
    if (run.openHitls.some((item) => item.nodeId === nodeId && item.status === "open")) {
      return;
    }
    const startedAt = host.nowIso();
    const schema = createMockHitlSchema(nodeId, locale);
    const request: HitlRequest = {
      id: host.nextHitlId(),
      runId,
      nodeId,
      schema,
      blocking: true,
      policy: "wait",
      status: "open",
      createdAt: startedAt,
    };
    const input: GraphWorkflowNodeIo = {
      summary: ioPreview(schema.prompt ?? schema.title ?? nodeId),
      detail: schema.prompt,
    };
    const withNode: GraphWorkflowRun = {
      ...run,
      status: "awaiting_input",
      openHitls: [...run.openHitls, request],
      nodeStates: {
        ...run.nodeStates,
        [nodeId]: {
          ...run.nodeStates[nodeId],
          status: "awaiting_input",
          startedAt,
          input,
        },
      },
      updatedAt: host.nowIso(),
    };
    host.setRun(withNode);
    host.notifyChanged(withNode);
    host.emit(runId, { type: "node_started", runId, nodeId });
    host.emit(runId, { type: "hitl_required", runId, request });
  }

  /**
   * Resolves one open HITL request by id and resumes the mock pump.
   * Sibling open gates stay in `openHitls` until the user answers them too.
   */
  function submitHitl(
    runId: string,
    requestId: string,
    payload: Record<string, unknown>,
  ): void {
    const run = host.getRun(runId);
    if (run === undefined) {
      throw new Error(`Unknown workflow run ${runId}`);
    }
    const request = run.openHitls.find(
      (item) => item.id === requestId && item.status === "open",
    );
    if (request === undefined) {
      throw new Error(`No open HITL request ${requestId} on run ${runId}`);
    }
    for (const field of request.schema.fields) {
      if (field.required !== true) {
        continue;
      }
      const value = payload[field.name];
      if (value === undefined || value === null || String(value).trim() === "") {
        throw new Error(`Missing required field ${field.name}`);
      }
      if (field.type === "select") {
        const allowed = new Set((field.options ?? []).map((option) => option.value));
        if (!allowed.has(String(value))) {
          throw new Error(`Invalid option for field ${field.name}`);
        }
      }
    }

    const nodeId = request.nodeId;
    const startedAt = run.nodeStates[nodeId]?.startedAt ?? host.nowIso();
    const finishedAt = host.nowIso();
    const durationMs = Math.max(
      Date.parse(finishedAt) - Date.parse(startedAt),
      1,
    );
    const tokenUsage = stubTokenUsage(nodeId);
    const remaining = run.openHitls.filter((item) => item.id !== requestId);
    const prev = run.nodeStates[nodeId];
    const resolved: GraphWorkflowRun = {
      ...run,
      status: remaining.length > 0 ? "awaiting_input" : "running",
      openHitls: remaining,
      nodeStates: {
        ...run.nodeStates,
        [nodeId]: {
          status: "succeeded",
          startedAt,
          finishedAt,
          durationMs,
          tokenUsage,
          input: prev?.input,
          output: hitlAnswerOutput(request.schema, payload),
        },
      },
      updatedAt: finishedAt,
    };
    host.setRun(resolved);
    host.notifyChanged(resolved);
    host.emit(runId, {
      type: "hitl_resolved",
      runId,
      requestId,
      nodeId,
      payload,
    });
    host.emit(runId, {
      type: "node_finished",
      runId,
      nodeId,
      status: "succeeded",
      durationMs,
      tokenUsage,
    });
    pump(runId);
  }
  function completeNode(
    runId: string,
    nodeId: string,
    startedAt: string,
    stepMs: number,
  ): void {
    const run = host.getRun(runId);
    if (run === undefined) {
      return;
    }
    const finishedAt = host.nowIso();
    const durationMs = Math.max(stepMs, 1);
    const node = run.definitionSnapshot.nodes.find((item) => item.id === nodeId);
    const tokenUsage = node && nodeKindUsesTokens(node.data.kind)
      ? stubTokenUsage(nodeId)
      : undefined;
    const prev = run.nodeStates[nodeId];
    patchNode(runId, nodeId, {
      status: "succeeded",
      startedAt,
      finishedAt,
      durationMs,
      tokenUsage,
      input: prev?.input,
      output: stubNodeOutput(run, nodeId),
    });
    host.emit(runId, {
      type: "node_finished",
      runId,
      nodeId,
      status: "succeeded",
      durationMs,
      tokenUsage,
    });

    if (node?.data.kind === "agent" || node?.data.kind === "output") {
      const artifact: WorkflowArtifact = {
        id: host.nextArtifactId(),
        runId,
        nodeId,
        kind: "markdown",
        title: node.data.title,
        body:
          node.data.kind === "output"
            ? `## ${node.data.title}\n\nMock run completed for **${run.name}**.`
            : `### ${node.data.title}\n\n${node.data.instruction}`,
        createdAt: finishedAt,
      };
      host.appendArtifact(artifact);
      host.emit(runId, { type: "artifact_added", runId, artifact });
    }
  }

  function patchNode(
    runId: string,
    nodeId: string,
    patch: GraphWorkflowNodeState,
  ): void {
    const run = host.getRun(runId);
    if (run === undefined) {
      return;
    }
    const updated: GraphWorkflowRun = {
      ...run,
      nodeStates: {
        ...run.nodeStates,
        [nodeId]: { ...run.nodeStates[nodeId], ...patch },
      },
      updatedAt: host.nowIso(),
    };
    host.setRun(updated);
    host.notifyChanged(updated);
  }

  function finishRun(
    runId: string,
    status: "succeeded" | "failed" | "cancelled",
  ): void {
    stop(runId);
    const run = host.getRun(runId);
    if (run === undefined) {
      return;
    }
    const finishedAt = host.nowIso();
    let totalTokens = 0;
    let durationMs = 0;
    for (const state of Object.values(run.nodeStates)) {
      totalTokens += state.tokenUsage?.totalTokens ?? 0;
      durationMs += state.durationMs ?? 0;
    }
    const totals = {
      durationMs,
      tokenUsage: totalTokens > 0 ? { totalTokens } : {},
    };
    const updated: GraphWorkflowRun = {
      ...run,
      status,
      totals,
      openHitls: [],
      updatedAt: finishedAt,
      finishedAt,
    };
    host.setRun(updated);
    host.notifyChanged(updated);
    host.emit(runId, { type: "run_finished", runId, status, totals });
  }

  /**
   * Begins execution from `pending` only (re-entrant start is a no-op).
   * HITL resume uses `submitHitl`, not this method.
   */
  function start(runId: string): void {
    const run = host.getRun(runId);
    if (run === undefined || run.status !== "pending") {
      return;
    }
    stop(runId);
    const plan = planMockExecution(
      run.definitionSnapshot,
      { kickoffInput: run.kickoffInput },
      pathPolicy,
    );
    plans.set(runId, plan);

    const nodeStates = { ...run.nodeStates };
    for (const nodeId of plan.skipped) {
      nodeStates[nodeId] = { status: "skipped" };
    }
    const started: GraphWorkflowRun = {
      ...run,
      status: "running",
      openHitls: [],
      nodeStates,
      updatedAt: host.nowIso(),
    };
    host.setRun(started);
    host.notifyChanged(started);
    host.emit(runId, { type: "run_started", runId });
    for (const nodeId of plan.skipped) {
      host.emit(runId, {
        type: "node_finished",
        runId,
        nodeId,
        status: "skipped",
      });
    }
    pump(runId);
  }

  /** Stops timers, marks active nodes cancelled, and emits run_finished. */
  function cancel(runId: string): void {
    stop(runId);
    const run = host.getRun(runId);
    if (run === undefined || isTerminal(run.status)) {
      return;
    }
    const finishedAt = host.nowIso();
    const nodeStates = { ...run.nodeStates };
    for (const [nodeId, state] of Object.entries(nodeStates)) {
      if (state.status === "running" || state.status === "awaiting_input") {
        nodeStates[nodeId] = { ...state, status: "cancelled", finishedAt };
      }
    }
    host.setRun({
      ...run,
      nodeStates,
      openHitls: [],
      updatedAt: finishedAt,
    });
    finishRun(runId, "cancelled");
  }

  return { start, stop, cancel, submitHitl };
}

function isTerminal(status: GraphWorkflowRun["status"]): boolean {
  return (
    status === "succeeded"
    || status === "failed"
    || status === "cancelled"
    || status === "partial_failed"
  );
}

function stubTokenUsage(nodeId: string): GraphWorkflowTokenUsage {
  return {
    inputTokens: 40 + nodeId.length * 3,
    outputTokens: 60 + nodeId.length * 2,
    totalTokens: 100 + nodeId.length * 5,
  };
}

/**
 * Full-graph topological order (does not apply condition exclusivity).
 * Prefer `planMockExecution` when simulating a run.
 */
export function executionOrder(workflow: DemoWorkflow): string[] {
  return topologicalOrder(
    workflow.nodes.map((node) => node.id),
    workflow.edges,
  );
}
