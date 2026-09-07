import {
  createDefaultMockPathPolicy,
  planMockExecution,
  topologicalOrder,
  type MockExecutionPlan,
  type MockPathPolicy,
} from "./mock-execution-plan";
import { validateWorkflowDefinition } from "./definition";
import type {
  GraphWorkflowNodeIo,
  GraphWorkflowNodeState,
  GraphWorkflowRun,
  HitlRequest,
  HitlSchema,
  WorkflowArtifact,
  WorkflowNodeConversationItem,
  WorkflowDefinition,
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
  upsertConversationItem: (item: WorkflowNodeConversationItem) => void;
  emit: (runId: string, event: WorkflowRunEvent) => void;
  notifyChanged: (run: GraphWorkflowRun) => void;
  nowIso: () => string;
  nextArtifactId: () => string;
  nextHitlId: () => string;
  nextConversationItemId: () => string;
}

/** Truncates text for glanceable I/O summaries. */
function ioPreview(text: string, max = 96): string {
  const trimmed = text.trim().replace(/\s+/g, " ");
  if (trimmed.length <= max) {
    return trimmed;
  }
  return `${trimmed.slice(0, max - 1)}…`;
}

/** Builds mock HITL schema for a human node (approval / feedback / clarify). */
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
    placeholder: en ? "Reply to the model’s question…" : "直接回复模型的问题…",
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
  const prompt = node?.data.agentConfig?.prompt;
  const kickoff = run.kickoffInput?.trim() ?? "";
  if (node?.data.kind === "agent" && kickoff !== "") {
    return {
      summary: ioPreview(kickoff),
      detail: prompt,
    };
  }
  return {
    summary: title,
    detail: node?.data.kind === "agent" ? prompt : undefined,
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
    };
  }
  if (kind === "tool") {
    return {
      summary: `Tool finished: ${node?.data.tool ?? title}`,
    };
  }
  return {
    summary: `Completed: ${title}`,
  };
}

/** Summarizes a HITL submit payload for node output. */
function hitlAnswerOutput(
  schema: HitlSchema,
  payload: Record<string, unknown>,
): GraphWorkflowNodeIo {
  const isSingleAnswerField =
    schema.fields.length === 1 && schema.fields[0]?.name === "answer";
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
      const label =
        field.options?.find((option) => option.value === text)?.label ?? text;
      parts.push(`${field.label}: ${label}`);
    } else {
      // Keep single-answer clarify responses chat-like ("xxx") while preserving
      // key/value readability for multi-field approvals and feedback forms.
      parts.push(isSingleAnswerField ? text : `${field.label}: ${text}`);
    }
  }
  const joined = parts.join(" · ");
  return {
    summary: ioPreview(joined !== "" ? joined : "Submitted"),
    detail: joined !== "" ? joined : undefined,
  };
}

/**
 * Mock executor over a frozen transport-neutral workflow snapshot.
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

  /** Stores one conversation item and emits its upsert event. */
  function publishConversationItem(
    runId: string,
    item: WorkflowNodeConversationItem,
  ): void {
    host.upsertConversationItem(item);
    host.emit(runId, {
      type: "node_conversation_item_upserted",
      runId,
      item,
    });
  }

  /** Appends one visible message line to a node-bound conversation. */
  function publishConversationMessage(
    runId: string,
    nodeId: string,
    sessionId: string,
    role: "user" | "assistant",
    markdown: string,
    timestamp: string,
  ): void {
    publishConversationItem(runId, {
      kind: "message",
      id: host.nextConversationItemId(),
      runId,
      nodeId,
      sessionId,
      role,
      markdown,
      status: "complete",
      createdAt: timestamp,
      updatedAt: timestamp,
    });
  }

  /** Resolves step length: node mockStepMs when positive, else engine default. */
  function stepMsFor(run: GraphWorkflowRun, nodeId: string): number {
    const node = run.definitionSnapshot.nodes.find(
      (item) => item.id === nodeId,
    );
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

  function timersFor(
    runId: string,
  ): Map<string, ReturnType<typeof setTimeout>> {
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
        // A non-taken condition branch is not part of the executed path: its nodes
        // stay idle and never gate their downstream siblings.
        return pred?.status === "succeeded" || plan.skipped.includes(predId);
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
        status === "succeeded" ||
        status === "failed" ||
        status === "cancelled" ||
        plan.skipped.includes(nodeId)
      );
    });
    if (
      allDone &&
      timersFor(runId).size === 0 &&
      latest.openHitls.length === 0
    ) {
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
    const node = run.definitionSnapshot.nodes.find(
      (item) => item.id === nodeId,
    );
    if (node?.data.kind === "human") {
      beginHitl(runId, nodeId);
      return;
    }

    const startedAt = host.nowIso();
    const stepMs = stepMsFor(run, nodeId);
    const input = stubNodeInput(run, nodeId);
    const sessionId = `workflow-node:${runId}:${nodeId}`;
    patchNode(runId, nodeId, {
      status: "running",
      sessionId,
      startedAt,
      input,
    });
    host.emit(runId, { type: "node_started", runId, nodeId });

    const inputText = input.detail?.trim() || input.summary.trim();
    if (inputText !== "") {
      const inputMessage: WorkflowNodeConversationItem = {
        kind: "message",
        id: host.nextConversationItemId(),
        runId,
        nodeId,
        sessionId,
        role: "user",
        markdown: inputText,
        status: "complete",
        createdAt: startedAt,
        updatedAt: startedAt,
      };
      host.upsertConversationItem(inputMessage);
      host.emit(runId, {
        type: "node_conversation_item_upserted",
        runId,
        item: inputMessage,
      });
    }

    // The mock keeps a little realistic activity in the projection so the UI
    // can demonstrate its collapsed disclosure without exposing it by default.
    if (node?.data.kind === "agent") {
      const thought: WorkflowNodeConversationItem = {
        kind: "activity",
        id: host.nextConversationItemId(),
        runId,
        nodeId,
        sessionId,
        activityKind: "thought",
        summary: "分析节点上下文",
        detail:
          "Mock thought: compare the instruction with the current workflow context.",
        status: "complete",
        createdAt: startedAt,
        updatedAt: startedAt,
      };
      host.upsertConversationItem(thought);
      host.emit(runId, {
        type: "node_conversation_item_upserted",
        runId,
        item: thought,
      });
      const tool: WorkflowNodeConversationItem = {
        kind: "activity",
        id: host.nextConversationItemId(),
        runId,
        nodeId,
        sessionId,
        activityKind: "tool",
        summary: "读取工作流上下文",
        detail: "Mock tool call: inspect upstream node outputs.",
        status: "complete",
        createdAt: startedAt,
        updatedAt: startedAt,
      };
      host.upsertConversationItem(tool);
      host.emit(runId, {
        type: "node_conversation_item_upserted",
        runId,
        item: tool,
      });
    }

    const timer = setTimeout(() => {
      timersFor(runId).delete(nodeId);
      const current = host.getRun(runId);
      if (current === undefined || current.status === "cancelled") {
        return;
      }
      completeNode(runId, nodeId, startedAt);
      pump(runId);
    }, stepMs);
    timersFor(runId).set(nodeId, timer);
  }

  /**
   * Pauses a human node for input until `submitHitl` (no timeout).
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
    if (
      run.openHitls.some(
        (item) => item.nodeId === nodeId && item.status === "open",
      )
    ) {
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
          sessionId: `workflow-node:${runId}:${nodeId}`,
          startedAt,
          input,
        },
      },
      updatedAt: host.nowIso(),
    };
    host.setRun(withNode);
    host.notifyChanged(withNode);
    host.emit(runId, { type: "node_started", runId, nodeId });
    const sessionId = `workflow-node:${runId}:${nodeId}`;
    for (const markdown of promptNodeContextMessages(nodeId, locale)) {
      publishConversationMessage(
        runId,
        nodeId,
        sessionId,
        "assistant",
        markdown,
        startedAt,
      );
    }
    publishConversationMessage(
      runId,
      nodeId,
      sessionId,
      "assistant",
      hitlQuestionMarkdown(schema, locale),
      startedAt,
    );
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
      if (
        value === undefined ||
        value === null ||
        String(value).trim() === ""
      ) {
        throw new Error(`Missing required field ${field.name}`);
      }
      if (field.type === "select") {
        const allowed = new Set(
          (field.options ?? []).map((option) => option.value),
        );
        if (!allowed.has(String(value))) {
          throw new Error(`Invalid option for field ${field.name}`);
        }
      }
    }

    const nodeId = request.nodeId;
    const startedAt = run.nodeStates[nodeId]?.startedAt ?? host.nowIso();
    const finishedAt = host.nowIso();
    const remaining = run.openHitls.filter((item) => item.id !== requestId);
    const prev = run.nodeStates[nodeId];
    const answer = hitlAnswerOutput(request.schema, payload);
    const sessionId = prev?.sessionId ?? `workflow-node:${runId}:${nodeId}`;
    const resolved: GraphWorkflowRun = {
      ...run,
      status: remaining.length > 0 ? "awaiting_input" : "running",
      openHitls: remaining,
      nodeStates: {
        ...run.nodeStates,
        [nodeId]: {
          status: "succeeded",
          sessionId,
          startedAt,
          finishedAt,
          input: prev?.input,
          output: answer,
        },
      },
      updatedAt: finishedAt,
    };
    host.setRun(resolved);
    host.notifyChanged(resolved);
    const userMessage: WorkflowNodeConversationItem = {
      kind: "message",
      id: host.nextConversationItemId(),
      runId,
      nodeId,
      sessionId,
      role: "user",
      markdown: answer.detail ?? answer.summary,
      status: "complete",
      createdAt: finishedAt,
      updatedAt: finishedAt,
    };
    publishConversationItem(runId, userMessage);
    const followUps = promptNodeFollowUpMessages(nodeId, locale);
    if (followUps.length > 0) {
      for (const markdown of followUps) {
        publishConversationMessage(
          runId,
          nodeId,
          sessionId,
          "assistant",
          markdown,
          finishedAt,
        );
      }
    } else {
      // Approval / generic human nodes need a visible ack so the session
      // projection changes after submit (not only the user's own bubble).
      publishConversationMessage(
        runId,
        nodeId,
        sessionId,
        "assistant",
        hitlAckMarkdown(answer, locale),
        finishedAt,
      );
    }
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
    });
    pump(runId);
  }
  function completeNode(
    runId: string,
    nodeId: string,
    startedAt: string,
  ): void {
    const run = host.getRun(runId);
    if (run === undefined) {
      return;
    }
    const finishedAt = host.nowIso();
    const node = run.definitionSnapshot.nodes.find(
      (item) => item.id === nodeId,
    );
    const prev = run.nodeStates[nodeId];
    patchNode(runId, nodeId, {
      status: "succeeded",
      startedAt,
      finishedAt,
      input: prev?.input,
      output: stubNodeOutput(run, nodeId),
    });
    host.emit(runId, {
      type: "node_finished",
      runId,
      nodeId,
      status: "succeeded",
    });

    if (node?.data.kind === "agent" || node?.data.kind === "output") {
      const instruction =
        node.data.agentConfig?.prompt ??
        node.data.description ??
        "节点已完成。";
      const markdown =
        node.data.kind === "output"
          ? markdownDemoOutput(node.data.title, run.name, locale)
          : markdownDemoAgentReply(
              node.data.title,
              instruction,
              runId,
              nodeId,
              locale,
            );
      const sessionId =
        run.nodeStates[nodeId]?.sessionId ?? `workflow-node:${runId}:${nodeId}`;
      publishConversationMessage(
        runId,
        nodeId,
        sessionId,
        "assistant",
        markdown,
        finishedAt,
      );
      const artifact: WorkflowArtifact = {
        id: host.nextArtifactId(),
        runId,
        nodeId,
        kind: "markdown",
        title: node.data.title,
        body: markdown,
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
    const updated: GraphWorkflowRun = {
      ...run,
      status,
      openHitls: [],
      updatedAt: finishedAt,
      finishedAt,
    };
    host.setRun(updated);
    host.notifyChanged(updated);
    host.emit(runId, { type: "run_finished", runId, status });
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

    // Non-taken condition branches have no node-run row, matching the persisted
    // backend model: those nodes stay idle instead of carrying a "skipped" status.
    const started: GraphWorkflowRun = {
      ...run,
      status: "running",
      openHitls: [],
      nodeStates: run.nodeStates,
      updatedAt: host.nowIso(),
    };
    host.setRun(started);
    host.notifyChanged(started);
    host.emit(runId, { type: "run_started", runId });
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

  /** Releases every timer owned by this adapter instance. */
  function dispose(): void {
    for (const runId of [...timers.keys()]) {
      stop(runId);
    }
  }

  return { start, stop, cancel, submitHitl, dispose };
}

/** Context messages shown before a HITL question so readers can decide with history. */
function promptNodeContextMessages(
  nodeId: string,
  locale: "zh-CN" | "en-US",
): string[] {
  const zh = locale === "zh-CN";
  if (nodeId === "understand") {
    return zh
      ? [
          `## 改动摘要

这次提交同时触及：

1. **鉴权中间件**（入口守卫 / 会话恢复）
2. **路由表**（动态参数与重定向）

> 在请你选择优先级前，先把已知事实对齐。`,
          `### 已观察到的信号

| 区域 | 风险 | 说明 |
| --- | --- | --- |
| Auth | 高 | \`requireAuth\` 分支条件有改动 |
| Router | 中 | 新增 \`:orgId\` 动态段 |
| Docs | 低 | README 尚未同步 |

\`\`\`ts
if (!session?.active) {
  return redirect("/login");
}
\`\`\``,
        ]
      : [
          `## Change summary

This commit touches both:

1. **Auth middleware** (entry guard / session restore)
2. **Route table** (dynamic params and redirects)

> Aligning known facts before asking you to prioritize.`,
          `### Signals so far

| Area | Risk | Note |
| --- | --- | --- |
| Auth | High | \`requireAuth\` branch conditions changed |
| Router | Medium | New \`:orgId\` dynamic segment |
| Docs | Low | README not updated yet |

\`\`\`ts
if (!session?.active) {
  return redirect("/login");
}
\`\`\``,
        ];
  }
  if (nodeId === "docs_pass") {
    return zh
      ? [
          `## 文档校对前置

索引已完成。接下来会核对：

- README 行为描述是否与实现一致
- 模块注释是否仍指向旧路径
- 示例代码是否可复制运行

\`\`\`bash
rg -n "legacy auth" docs README.md
\`\`\``,
        ]
      : [
          `## Docs pass preamble

Index is ready. Next checks:

- README behavior vs implementation
- module comments still pointing at old paths
- examples still copy-pasteable

\`\`\`bash
rg -n "legacy auth" docs README.md
\`\`\``,
        ];
  }
  return [];
}

/** Formats the HITL prompt as Markdown so the node conversation showcases rendering. */
function hitlQuestionMarkdown(
  schema: HitlSchema,
  locale: "zh-CN" | "en-US",
): string {
  const zh = locale === "zh-CN";
  const prompt = schema.prompt?.trim();
  const title = schema.title?.trim();
  if (prompt !== undefined && prompt !== "") {
    return zh
      ? `### 需要你确认\n\n${prompt}\n\n请在下方提交后继续。`
      : `### Input needed\n\n${prompt}\n\nSubmit below to continue.`;
  }
  if (title !== undefined && title !== "") {
    return `### ${title}`;
  }
  return zh ? "### 需要你确认" : "### Input needed";
}

/** Short assistant ack so approval gates visibly update the node session. */
function hitlAckMarkdown(
  answer: GraphWorkflowNodeIo,
  locale: "zh-CN" | "en-US",
): string {
  const body = (answer.detail ?? answer.summary).trim();
  if (locale === "zh-CN") {
    return `已收到你的确认：\n\n> ${body}\n\n我会按这个选择继续后续步骤。`;
  }
  return `Got your confirmation:\n\n> ${body}\n\nContinuing with the next steps.`;
}

/** Adds richer multi-message prompt follow-ups for anchor-navigation demos. */
function promptNodeFollowUpMessages(
  nodeId: string,
  locale: "zh-CN" | "en-US",
): string[] {
  const zh = locale === "zh-CN";
  if (nodeId === "understand") {
    return zh
      ? [
          "我先按你的选择把核查顺序固定为：**权限边界 -> 路由回归面**。这样可以先锁住高风险区域，再扩散到影响路径。",
          `### 权限边界快照

- 鉴权中间件入口与退出条件
- 匿名与登录态分叉
- 管理权限提升链路

\`\`\`ts
export function requireRole(role: Role) {
  return (ctx) => ctx.user.roles.includes(role);
}
\`\`\``,
          `### 路由回归面快照

| 检查项 | 状态 |
| --- | --- |
| 新增/删除路由映射 | 待核 |
| 动态参数与守卫组合 | 待核 |
| 旧链接兼容与重定向 | 待核 |

> 建议优先补一条 \`/settings/:orgId\` 的登录态回归。`,
          "下一步我会先输出边界风险点，再附路由覆盖建议，方便你快速判定是否需要加回归测试。",
        ]
      : [
          "I will lock the review order to **permission boundaries -> route regression surface** so high-risk checks land first.",
          `### Permission boundary snapshot

- auth middleware entry and exit gates
- anonymous vs signed-in branch split
- privilege escalation chain

\`\`\`ts
export function requireRole(role: Role) {
  return (ctx) => ctx.user.roles.includes(role);
}
\`\`\``,
          `### Route regression snapshot

| Check | Status |
| --- | --- |
| added/removed route mappings | pending |
| dynamic params with guards | pending |
| legacy link redirects | pending |

> Prefer a signed-in regression for \`/settings/:orgId\` first.`,
          "Next I will report boundary risks first, then attach route coverage suggestions for quick test planning.",
        ];
  }
  if (nodeId === "docs_pass") {
    return zh
      ? [
          "已收到你的反馈，我会按“实现变化 -> 文档变化 -> 示例变化”的顺序过一遍，确保阅读路径一致。",
          `### 文档核对清单

1. README 的行为描述
2. 模块注释与边界说明
3. 示例与截图的时效性

\`\`\`diff
- 旧鉴权入口：middleware/auth.ts
+ 新鉴权入口：middleware/session-guard.ts
\`\`\``,
          "若发现术语不一致，我会优先给出替换建议，并标注是否影响外部使用者理解。",
          "完成后会附一个最小更新补丁建议，避免文档改动过大影响评审效率。",
        ]
      : [
          "Got it. I will review docs in order: implementation changes -> docs changes -> examples.",
          `### Docs review checklist

1. README behavior descriptions
2. module comments and boundaries
3. examples and screenshots freshness

\`\`\`diff
- old auth entry: middleware/auth.ts
+ new auth entry: middleware/session-guard.ts
\`\`\``,
          "If terminology drifts, I will propose direct replacements and note user-facing impact.",
          "I will end with a minimal patch proposal to keep review focused.",
        ];
  }
  return [];
}

/** Rich Markdown agent reply used by mock completion artifacts and conversation. */
function markdownDemoAgentReply(
  title: string,
  instruction: string,
  runId: string,
  nodeId: string,
  locale: "zh-CN" | "en-US",
): string {
  const zh = locale === "zh-CN";
  return zh
    ? `### ${title}

${instruction}

#### Mock 结论

- 已完成上下文检查，并保留一条可追溯的正式结论。
- 结果支持 **粗体**、列表、表格和代码块，长内容会在卡片内滚动。

#### 检查记录

| 项目 | 状态 |
| --- | --- |
| 节点输入 | 已接收 |
| Agent 正式回复 | 已生成 |
| 工具与思考 | 已折叠 |

\`\`\`text
session: workflow-node:${runId}:${nodeId}
projection: visible-messages + collapsed-activity
\`\`\`

> 如果接入真实 session，前端会继续只展示这类正式消息。`
    : `### ${title}

${instruction}

#### Mock conclusion

- Context checks completed with a traceable formal reply.
- Rendering covers **bold**, lists, tables, and fenced code; long bodies scroll in-card.

#### Checklist

| Item | Status |
| --- | --- |
| Node input | received |
| Agent reply | generated |
| Tools / thoughts | collapsed |

\`\`\`text
session: workflow-node:${runId}:${nodeId}
projection: visible-messages + collapsed-activity
\`\`\`

> A real session adapter can keep feeding the same visible-message projection.`;
}

/** Short Markdown completion body for output nodes. */
function markdownDemoOutput(
  title: string,
  runName: string,
  locale: "zh-CN" | "en-US",
): string {
  const zh = locale === "zh-CN";
  return zh
    ? `## ${title}

Mock run **${runName}** 已完成。

- 摘要已生成
- 产物可在检查器中打开`
    : `## ${title}

Mock run **${runName}** completed.

- Summary generated
- Artifacts available in the inspector`;
}

function isTerminal(status: GraphWorkflowRun["status"]): boolean {
  return (
    status === "succeeded" || status === "failed" || status === "cancelled"
  );
}

/**
 * Full-graph topological order (does not apply condition exclusivity).
 * Prefer `planMockExecution` when simulating a run.
 */
export function executionOrder(workflow: WorkflowDefinition): string[] {
  validateWorkflowDefinition(workflow);
  return topologicalOrder(
    workflow.nodes.map((node) => node.id),
    workflow.edges,
  );
}
