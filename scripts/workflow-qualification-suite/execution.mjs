const terminalStatuses = new Set(["succeeded", "failed", "cancelled"]);
const requiredKinds = ["condition", "aggregator", "iteration", "loop"];
const defaultSleep = (milliseconds) =>
  new Promise((resolve) => setTimeout(resolve, milliseconds));

/** Polls one run with a deadline; an interactive pause needs cancellation by this unattended runner. */
async function pollRun(
  session,
  runId,
  { now, sleep, timeoutMs, pollIntervalMs, signal },
) {
  const deadline = now() + timeoutMs;
  let detail = null;
  while (now() < deadline) {
    if (signal?.aborted) throw new Error("Qualification interrupted.");
    await sleep(Math.min(pollIntervalMs, deadline - now()));
    if (signal?.aborted) throw new Error("Qualification interrupted.");
    detail = await session.invoke("get_workflow_run", { runId });
    if (now() > deadline) return { status: "timeout", detail };
    const status = detail?.run?.status;
    if (terminalStatuses.has(status) || status === "awaitingInput") {
      return { status, detail };
    }
    if (status !== "running" && status !== "pending") {
      throw new Error(
        `Run ${runId} returned an unknown status: ${String(status)}`,
      );
    }
  }
  return { status: "timeout", detail };
}

/** Runs one published graph and releases only the definitions and runs this invocation owns. */
export async function runScenario(
  session,
  workflow,
  workspaceId,
  options = {},
) {
  if (typeof workspaceId !== "string" || workspaceId.trim() === "") {
    throw new Error(
      "ORA_QUALIFICATION_WORKSPACE_ID must name a disposable workspace.",
    );
  }
  const {
    now = Date.now,
    sleep = defaultSleep,
    // 300 s keeps genuine model slowness distinguishable from an engine stall; the
    // qualification bar still requires every scenario to finish well inside this ceiling.
    timeoutMs = 300_000,
    pollIntervalMs = 1_000,
    cancelTimeoutMs = 30_000,
    signal,
  } = options;
  for (const [name, value] of Object.entries({
    timeoutMs,
    pollIntervalMs,
    cancelTimeoutMs,
  })) {
    if (!Number.isFinite(value) || value <= 0) {
      throw new Error(`${name} must be positive.`);
    }
  }

  const startTime = now();
  let executionStart = null;
  let workflowId = null;
  let runId = null;
  let mayBeActive = false;
  let detail = null;
  let status = "error";
  let error = null;
  const cleanupErrors = [];

  try {
    if (signal?.aborted) throw new Error("Qualification interrupted.");
    const created = await session.invoke("create_workflow", {
      name: `${workflow.name}-${startTime}`,
      graph: JSON.stringify(workflow.graph),
    });
    workflowId = created.workflow.id;
    await session.invoke("publish_workflow", {
      workflowId,
      version: "qualification",
    });
    const createdRun = await session.invoke("create_workflow_run", {
      workspaceId,
      workflowId,
      locale: "zh-CN",
    });
    runId = createdRun.run.id;
    // Mark before the request: a lost response does not prove the engine stayed idle.
    mayBeActive = true;
    executionStart = now();
    await session.invoke("start_workflow_run", { runId });
    ({ status, detail } = await pollRun(session, runId, {
      now,
      sleep,
      timeoutMs,
      pollIntervalMs,
      signal,
    }));
    mayBeActive = !terminalStatuses.has(detail?.run?.status);
    error =
      status === "timeout"
        ? `Run exceeded the ${timeoutMs} ms deadline.`
        : status === "awaitingInput"
          ? "Run paused for human input during unattended qualification."
          : (detail?.run?.error ?? null);
  } catch (failure) {
    error = failure instanceof Error ? failure.message : String(failure);
  }

  const observedExecutionMs =
    executionStart === null ? 0 : now() - executionStart;
  const startedAt = detail?.run?.startedAt;
  const finishedAt = detail?.run?.finishedAt;
  const hasPersistedDuration =
    Number.isFinite(startedAt) &&
    Number.isFinite(finishedAt) &&
    finishedAt >= startedAt;
  const executionDurationMs = hasPersistedDuration
    ? finishedAt - startedAt
    : observedExecutionMs;

  if (runId !== null && mayBeActive) {
    try {
      await session.invoke("cancel_workflow_run", { runId });
      const settled = await pollRun(session, runId, {
        now,
        sleep,
        timeoutMs: cancelTimeoutMs,
        pollIntervalMs,
      });
      if (!terminalStatuses.has(settled.status)) {
        throw new Error(`Cancellation did not settle run ${runId}.`);
      }
      mayBeActive = false;
    } catch (failure) {
      cleanupErrors.push(
        `Cancel ${runId}: ${
          failure instanceof Error ? failure.message : String(failure)
        }`,
      );
    }
  }
  // A failed cancellation must halt the suite rather than overlap new agents with an unknown live run.
  if (!mayBeActive) {
    if (runId !== null) {
      try {
        await session.invoke("delete_workflow_run", { runId });
      } catch (failure) {
        cleanupErrors.push(
          `Delete ${runId}: ${
            failure instanceof Error ? failure.message : String(failure)
          }`,
        );
      }
    }
    if (workflowId !== null) {
      try {
        await session.invoke("delete_workflow", { workflowId });
      } catch (failure) {
        cleanupErrors.push(
          `Delete ${workflowId}: ${
            failure instanceof Error ? failure.message : String(failure)
          }`,
        );
      }
    }
  }

  const nodes = detail?.nodes ?? [];
  const coveredKinds = requiredKinds.filter((kind) =>
    nodes.some((node) => node.nodeType === kind && node.status === "succeeded"),
  );
  const qualified =
    status === "succeeded" &&
    coveredKinds.length === requiredKinds.length &&
    cleanupErrors.length === 0;
  return {
    index: workflow.index,
    name: workflow.name,
    category: workflow.category,
    workflowId,
    runId,
    status,
    durationMs: now() - startTime,
    executionDurationMs,
    executionDurationSource: hasPersistedDuration ? "persisted" : "observed",
    qualified,
    nodeCount: nodes.length,
    definitionNodeCount: workflow.graph.nodes.length,
    coveredKinds,
    nodeSummary: nodes.map((node) => ({
      id: node.nodeId,
      type: node.nodeType,
      status: node.status,
    })),
    output: detail?.run?.output ?? null,
    error,
    cleanupComplete: !mayBeActive && cleanupErrors.length === 0,
    cleanupErrors,
  };
}

/** Qualifies one complete long run; short repeated runs cannot stand in for its duration. */
export async function runEnduranceStress(
  session,
  workflow,
  workspaceId,
  options = {},
) {
  const result = await runScenario(session, workflow, workspaceId, {
    timeoutMs: 75 * 60 * 1_000,
    pollIntervalMs: 4_000,
    ...options,
  });
  const meetsDuration = result.executionDurationMs >= 60 * 60 * 1_000;
  const meetsNodeCount = result.definitionNodeCount >= 15;
  return {
    ...result,
    durationMinutes: (result.executionDurationMs / 60_000).toFixed(2),
    meetsDuration,
    meetsNodeCount,
    qualified: result.qualified && meetsDuration && meetsNodeCount,
  };
}

/**
 * Qualifies one super-long workflow from the W102-W112 batch. The class bars keep the
 * original stress vocabulary (>= 15 nodes, >= 10 deep nodes) and add a >= 15 minute
 * measured duration so a super-long run cannot be a fast graph in disguise.
 */
export async function runSuperLong(
  session,
  workflow,
  workspaceId,
  options = {},
) {
  const result = await runScenario(session, workflow, workspaceId, {
    timeoutMs: 100 * 60 * 1_000,
    pollIntervalMs: 5_000,
    ...options,
  });
  const meetsDuration = result.executionDurationMs >= 15 * 60 * 1_000;
  const meetsNodeCount = result.definitionNodeCount >= 15;
  const meetsDeepNodes = (workflow.deepNodeCount ?? 0) >= 10;
  return {
    ...result,
    durationMinutes: (result.executionDurationMs / 60_000).toFixed(2),
    deepNodeCount: workflow.deepNodeCount ?? null,
    meetsDuration,
    meetsNodeCount,
    meetsDeepNodes,
    qualified:
      result.qualified && meetsDuration && meetsNodeCount && meetsDeepNodes,
  };
}
