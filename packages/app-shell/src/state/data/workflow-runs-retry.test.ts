import { describe, expect, it } from "vitest";
import type { WorkflowNodeAttemptFailure } from "@ora/workflow-runtime";
import "../../i18n/i18n-instance";
import type { PersistedFailedAttempt } from "./workflow-run-retry";
import { buildDisplayRun } from "./workflow-runs";

type RunDetail = Parameters<typeof buildDisplayRun>[0];
type NodeRow = RunDetail["nodes"][number];

const ROOT_SCOPE = "root:run-1";
const BASE_MS = 1_700_000_000_000;

/** Backend unix millis `offset` ms after BASE_MS. */
function ms(offset: number): bigint {
  return BigInt(BASE_MS + offset);
}

/** The adapter's ISO projection of `ms(offset)`. */
function iso(offset: number): string {
  return new Date(BASE_MS + offset).toISOString();
}

/** `start` fans out to two agent nodes that run in parallel. */
const PARALLEL_GRAPH = JSON.stringify({
  nodes: [
    {
      id: "start",
      type: "workflow",
      position: { x: 0, y: 0 },
      data: { kind: "start", title: "开始", description: "" },
    },
    {
      id: "explore",
      type: "workflow",
      position: { x: 200, y: 0 },
      data: { kind: "agent", title: "探索", description: "" },
    },
    {
      id: "review",
      type: "workflow",
      position: { x: 200, y: 160 },
      data: { kind: "agent", title: "审查", description: "" },
    },
  ],
  edges: [
    { id: "e1", source: "start", target: "explore" },
    { id: "e2", source: "start", target: "review" },
  ],
  viewport: { x: 32, y: 32, zoom: 1 },
  description: "审查流程",
});

/** The region member `fix` runs once per round inside the iteration node `iter`. */
const ITERATION_GRAPH = JSON.stringify({
  nodes: [
    {
      id: "start",
      type: "workflow",
      position: { x: 0, y: 0 },
      data: {
        kind: "start",
        title: "开始",
        description: "",
        inputVariables: [{ name: "prs", valueType: "array[object]" }],
      },
    },
    {
      id: "iter",
      type: "workflow",
      position: { x: 200, y: 0 },
      data: {
        kind: "iteration",
        title: "迭代",
        description: "",
        iterationConfig: {
          iteratorSelector: ["start", "prs"],
          collectSelector: ["fix", "output"],
          errorStrategy: "continue",
          maxIterations: 10,
        },
      },
    },
    {
      id: "fix",
      type: "workflow",
      parentId: "iter",
      position: { x: 40, y: 160 },
      data: { kind: "agent", title: "修复", description: "" },
    },
  ],
  edges: [
    { id: "e1", source: "start", target: "iter" },
    { id: "e2", source: "iter", target: "fix" },
  ],
  viewport: { x: 0, y: 0, zoom: 1 },
  description: "迭代流程",
});

/** The Loop body agent `child` runs once per Loop round, each round in its own scope. */
const LOOP_GRAPH = JSON.stringify({
  nodes: [
    {
      id: "start",
      type: "workflow",
      position: { x: 0, y: 0 },
      data: { kind: "start", title: "开始", description: "" },
    },
    {
      id: "loop-1",
      type: "workflow",
      position: { x: 200, y: 0 },
      data: { kind: "loop", title: "循环", description: "" },
    },
    {
      id: "child",
      type: "workflow",
      parentId: "loop-1",
      position: { x: 80, y: 100 },
      data: {
        kind: "agent",
        title: "改进",
        description: "",
        containerId: "loop-1",
      },
    },
  ],
  edges: [{ id: "e1", source: "start", target: "loop-1" }],
  viewport: { x: 0, y: 0, zoom: 1 },
});

function runDetail(
  nodes: NodeRow[],
  extra: Partial<Omit<RunDetail, "nodes">> = {},
): RunDetail {
  return {
    run: {
      id: "run-1",
      workflowId: "workflow-a",
      status: "running",
      state: '{"current_nodes":["explore"]}',
      input: null,
      startedAt: ms(0),
      finishedAt: null,
      createdAt: ms(0),
      updatedAt: ms(90_000),
    },
    name: "审查流程 1",
    projectId: "p1",
    variables: [],
    conditionDecisions: {},
    nodes,
    ...extra,
  };
}

function nodeRow(nodeId: string, overrides: Partial<NodeRow> = {}): NodeRow {
  return {
    id: `node-run-${nodeId}`,
    scopeId: ROOT_SCOPE,
    nodeId,
    status: "succeeded",
    startedAt: null,
    finishedAt: null,
    error: null,
    output: null,
    payload: null,
    sessionId: null,
    ...overrides,
  };
}

const START_ROW = nodeRow("start", {
  startedAt: ms(0),
  finishedAt: ms(1_000),
});

/** `payload.retry_wait` of a row that will run as attempt 3 after retry 2 of 3. */
const RETRY_WAIT_ATTEMPT_3 = {
  attempt: 3,
  max_attempt: 4,
  retry: 2,
  max_retries: 3,
  delay_ms: 4_000,
  scheduled_at: BASE_MS + 30_000,
  due_at: BASE_MS + 34_000,
  previous_node_run_id: "explore-attempt-2",
};

/** How the adapter shows `RETRY_WAIT_ATTEMPT_3`. */
const SHOWN_RETRY_WAIT_ATTEMPT_3 = {
  attempt: 3,
  maxAttempt: 4,
  retry: 2,
  maxRetries: 3,
  delayMs: 4_000,
  scheduledAt: BASE_MS + 30_000,
  dueAt: BASE_MS + 34_000,
};

function payload(value: Record<string, unknown>): string {
  return JSON.stringify(value);
}

/** A failed session attempt that ran from `at` to `at + 4s` and was recorded 1s later. */
type AttemptFixture = {
  id: string;
  nodeId: string;
  attempt: number;
  at: number;
};

function persistedAttempt(
  fixture: AttemptFixture,
  place: { scopeId?: string; iteration?: number } = {},
): PersistedFailedAttempt {
  return {
    nodeRunId: fixture.id,
    nodeId: fixture.nodeId,
    scopeId: place.scopeId ?? ROOT_SCOPE,
    iteration: place.iteration ?? null,
    sessionId: `session-${fixture.id}`,
    attempt: fixture.attempt,
    kind: "session",
    message: `${fixture.nodeId} 第 ${fixture.attempt} 次会话失败`,
    sourceChain: ["agent session failed", "prompt timed out"],
    recordedAt: ms(fixture.at + 5_000),
    startedAt: ms(fixture.at),
    finishedAt: ms(fixture.at + 4_000),
  };
}

function shownAttempt(
  fixture: AttemptFixture,
  extra: Partial<WorkflowNodeAttemptFailure> = {},
): WorkflowNodeAttemptFailure {
  return {
    nodeRunId: fixture.id,
    attempt: fixture.attempt,
    kind: "session",
    errorMessage: `${fixture.nodeId} 第 ${fixture.attempt} 次会话失败`,
    sourceChain: ["agent session failed", "prompt timed out"],
    recordedAt: BASE_MS + fixture.at + 5_000,
    startedAt: iso(fixture.at),
    finishedAt: iso(fixture.at + 4_000),
    sessionId: `session-${fixture.id}`,
    ...extra,
  };
}

const EXPLORE_1: AttemptFixture = {
  id: "explore-attempt-1",
  nodeId: "explore",
  attempt: 1,
  at: 10_000,
};
const EXPLORE_2: AttemptFixture = {
  id: "explore-attempt-2",
  nodeId: "explore",
  attempt: 2,
  at: 20_000,
};
const REVIEW_1: AttemptFixture = {
  id: "review-attempt-1",
  nodeId: "review",
  attempt: 1,
  at: 12_000,
};

describe("buildDisplayRun retry status", () => {
  it("projects a running row with no start time and a complete retry_wait to retry_waiting", () => {
    const display = buildDisplayRun(
      runDetail([
        START_ROW,
        nodeRow("explore", {
          id: "explore-attempt-3",
          status: "running",
          payload: payload({
            retry_wait: RETRY_WAIT_ATTEMPT_3,
            auto_retry: { retry: 2, max_retries: 3 },
          }),
        }),
      ]),
      PARALLEL_GRAPH,
    );

    expect(display.nodeStates.explore).toStrictEqual({
      status: "retry_waiting",
      retryWait: SHOWN_RETRY_WAIT_ATTEMPT_3,
      autoRetry: { retry: 2, maxRetries: 3 },
    });
    expect(display.status).toBe("running");
  });

  it("keeps a started row running and drops a leftover retry_wait", () => {
    const display = buildDisplayRun(
      runDetail([
        START_ROW,
        nodeRow("explore", {
          id: "explore-attempt-3",
          status: "running",
          startedAt: ms(34_100),
          sessionId: "session-explore-3",
          payload: payload({
            retry_wait: RETRY_WAIT_ATTEMPT_3,
            auto_retry: { retry: 2, max_retries: 3 },
          }),
        }),
      ]),
      PARALLEL_GRAPH,
    );

    expect(display.nodeStates.explore).toStrictEqual({
      status: "running",
      sessionId: "session-explore-3",
      startedAt: iso(34_100),
      autoRetry: { retry: 2, maxRetries: 3 },
    });
  });

  it.each([
    ["missing due_at", { ...RETRY_WAIT_ATTEMPT_3, due_at: undefined }] as const,
    ["a string attempt", { ...RETRY_WAIT_ATTEMPT_3, attempt: "3" }] as const,
    ["a fractional delay", { ...RETRY_WAIT_ATTEMPT_3, delay_ms: 1.5 }] as const,
    ["null", null] as const,
    ["an array", [3, 4, 2, 3, 4_000]] as const,
  ])(
    "keeps an unstarted running row running when retry_wait is %s",
    (_label, retryWait) => {
      const display = buildDisplayRun(
        runDetail([
          START_ROW,
          nodeRow("explore", {
            status: "running",
            payload: payload({
              retry_wait: retryWait,
              auto_retry: { retry: 2, max_retries: 3 },
            }),
          }),
        ]),
        PARALLEL_GRAPH,
      );

      expect(display.nodeStates.explore).toStrictEqual({
        status: "running",
        autoRetry: { retry: 2, maxRetries: 3 },
      });
    },
  );

  it("keeps a pending row awaiting_input even when its payload carries a retry_wait", () => {
    const display = buildDisplayRun(
      runDetail([
        START_ROW,
        nodeRow("explore", {
          status: "pending",
          payload: payload({ retry_wait: RETRY_WAIT_ATTEMPT_3 }),
        }),
      ]),
      PARALLEL_GRAPH,
    );

    expect(display.nodeStates.explore).toStrictEqual({
      status: "awaiting_input",
    });
  });
});

describe("buildDisplayRun retry outcome", () => {
  it("flags a waiting row the run abandoned with the backend's retry_abandoned reason", () => {
    const display = buildDisplayRun(
      runDetail(
        [
          START_ROW,
          nodeRow("explore", {
            status: "cancelled",
            finishedAt: ms(32_000),
            error: '{"reason":"retry_abandoned"}',
            payload: payload({
              retry_wait: RETRY_WAIT_ATTEMPT_3,
              auto_retry: { retry: 2, max_retries: 3 },
            }),
          }),
        ],
        {
          run: {
            ...runDetail([]).run,
            status: "failed",
            state: null,
            finishedAt: ms(32_000),
          },
        },
      ),
      PARALLEL_GRAPH,
    );

    expect(display.nodeStates.explore).toStrictEqual({
      status: "cancelled",
      finishedAt: iso(32_000),
      errorMessage: '{"reason":"retry_abandoned"}',
      autoRetry: { retry: 2, maxRetries: 3 },
      retryAbandoned: true,
    });
  });

  it("does not flag a waiting row the user cancelled", () => {
    const display = buildDisplayRun(
      runDetail([
        START_ROW,
        nodeRow("explore", {
          status: "cancelled",
          finishedAt: ms(32_000),
          payload: payload({
            retry_wait: RETRY_WAIT_ATTEMPT_3,
            auto_retry: { retry: 2, max_retries: 3 },
          }),
        }),
      ]),
      PARALLEL_GRAPH,
    );

    expect(display.nodeStates.explore).toStrictEqual({
      status: "cancelled",
      finishedAt: iso(32_000),
      autoRetry: { retry: 2, maxRetries: 3 },
    });
  });

  it("keeps auto_retry on a failed row so the inspector can say how often it was retried", () => {
    const display = buildDisplayRun(
      runDetail(
        [
          START_ROW,
          nodeRow("explore", {
            id: "explore-attempt-3",
            status: "failed",
            sessionId: "session-explore-3",
            startedAt: ms(34_000),
            finishedAt: ms(40_000),
            error: "agent session failed",
            payload: payload({
              auto_retry: { retry: 2, max_retries: 3 },
              error_detail: {
                kind: "session",
                message: "agent session failed",
                source_chain: ["agent session failed", "prompt timed out"],
                attempt: 3,
                resumable: true,
                injects_previous_failure: false,
                recorded_at: BASE_MS + 40_000,
              },
            }),
          }),
        ],
        {
          failedAttempts: [
            persistedAttempt(EXPLORE_1),
            persistedAttempt(EXPLORE_2),
          ],
        },
      ),
      PARALLEL_GRAPH,
    );

    // The owner's attempt number comes from error_detail.attempt (3), retry 2 walks back two.
    expect(display.nodeStates.explore).toStrictEqual({
      status: "failed",
      sessionId: "session-explore-3",
      startedAt: iso(34_000),
      finishedAt: iso(40_000),
      errorMessage: "agent session failed",
      errorDetail: {
        kind: "session",
        message: "agent session failed",
        sourceChain: ["agent session failed", "prompt timed out"],
        attempt: 3,
        resumable: true,
        injectsPreviousFailure: false,
        recordedAt: BASE_MS + 40_000,
      },
      autoRetry: { retry: 2, maxRetries: 3 },
      failedAttempts: [
        shownAttempt(EXPLORE_1, { replacedBy: "automatic_retry" }),
        shownAttempt(EXPLORE_2, { replacedBy: "automatic_retry" }),
      ],
    });
  });
});

describe("buildDisplayRun retries that never started", () => {
  // Attempt 2 of `explore` was scheduled by retry 1 and never started; its predecessor (attempt 1)
  // must not be shown as retried, whatever ended the wait.
  const unstarted = (
    overrides: Partial<NodeRow>,
    retryWait: Record<string, unknown> | undefined,
  ) =>
    buildDisplayRun(
      runDetail(
        [
          START_ROW,
          nodeRow("explore", {
            finishedAt: ms(32_000),
            ...overrides,
            payload: payload({
              ...(retryWait !== undefined ? { retry_wait: retryWait } : {}),
              auto_retry: { retry: 1, max_retries: 3 },
            }),
          }),
        ],
        { failedAttempts: [persistedAttempt(EXPLORE_1)] },
      ),
      PARALLEL_GRAPH,
    ).nodeStates.explore?.failedAttempts;

  const WAIT_ATTEMPT_2 = {
    ...RETRY_WAIT_ATTEMPT_3,
    attempt: 2,
    retry: 1,
  };

  it.each([
    ["still waiting", { status: "running", finishedAt: null }, WAIT_ATTEMPT_2],
    ["cancelled by the user while waiting", { status: "cancelled" }, undefined],
    [
      "abandoned by a run failure while waiting",
      { status: "cancelled", error: '{"reason":"retry_abandoned"}' },
      undefined,
    ],
    [
      "failed by an app restart while waiting",
      { status: "failed", error: '{"reason":"interrupted_by_restart"}' },
      undefined,
    ],
  ] as const)(
    "marks the attempt before a retry %s as scheduled, not retried",
    (_label, overrides, retryWait) => {
      expect(unstarted(overrides, retryWait)).toStrictEqual([
        shownAttempt(EXPLORE_1, { replacedBy: "automatic_retry_scheduled" }),
      ]);
    },
  );

  it("marks it as retried once the retry started", () => {
    expect(
      unstarted({ status: "failed", startedAt: ms(31_000) }, undefined),
    ).toStrictEqual([
      shownAttempt(EXPLORE_1, { replacedBy: "automatic_retry" }),
    ]);
  });
});

describe("buildDisplayRun failed attempts", () => {
  it("attaches each node's failed attempts oldest first with replacement marks", () => {
    const display = buildDisplayRun(
      runDetail(
        [
          START_ROW,
          nodeRow("explore", {
            status: "running",
            payload: payload({
              retry_wait: RETRY_WAIT_ATTEMPT_3,
              auto_retry: { retry: 2, max_retries: 3 },
            }),
          }),
          // Resumed by hand after attempt 1 and failed again as attempt 2.
          nodeRow("review", {
            status: "failed",
            startedAt: ms(25_000),
            finishedAt: ms(29_000),
            error: "review failed",
            payload: payload({
              error_detail: {
                kind: "session",
                message: "review failed",
                attempt: 2,
              },
            }),
          }),
        ],
        {
          failedAttempts: [
            persistedAttempt(EXPLORE_1),
            persistedAttempt(REVIEW_1),
            persistedAttempt(EXPLORE_2),
          ],
        },
      ),
      PARALLEL_GRAPH,
    );

    expect(display.nodeStates.explore.failedAttempts).toStrictEqual([
      shownAttempt(EXPLORE_1, { replacedBy: "automatic_retry" }),
      shownAttempt(EXPLORE_2, { replacedBy: "automatic_retry_scheduled" }),
    ]);
    expect(display.nodeStates.review.failedAttempts).toStrictEqual([
      shownAttempt(REVIEW_1, { replacedBy: "manual_resume" }),
    ]);
    expect(display.nodeStates.start).toStrictEqual({
      status: "succeeded",
      startedAt: iso(0),
      finishedAt: iso(1_000),
    });
  });

  it("puts region attempts on the matching round of roundStates", () => {
    const fixRound0: AttemptFixture = {
      id: "fix-round-0-attempt-1",
      nodeId: "fix",
      attempt: 1,
      at: 10_000,
    };
    const fixRound1: AttemptFixture = {
      id: "fix-round-1-attempt-1",
      nodeId: "fix",
      attempt: 1,
      at: 30_000,
    };
    const display = buildDisplayRun(
      runDetail(
        [
          START_ROW,
          nodeRow("iter", { status: "running", startedAt: ms(2_000) }),
          nodeRow("fix", {
            id: "fix-round-0-attempt-2",
            iteration: 0,
            status: "succeeded",
            sessionId: "session-fix-round-0-2",
            startedAt: ms(20_000),
            finishedAt: ms(25_000),
            payload: payload({ auto_retry: { retry: 1, max_retries: 2 } }),
          }),
          nodeRow("fix", {
            id: "fix-round-1-attempt-2",
            iteration: 1,
            status: "running",
            payload: payload({
              retry_wait: {
                attempt: 2,
                max_attempt: 3,
                retry: 1,
                max_retries: 2,
                delay_ms: 2_000,
                scheduled_at: BASE_MS + 35_000,
                due_at: BASE_MS + 37_000,
                previous_node_run_id: "fix-round-1-attempt-1",
              },
              auto_retry: { retry: 1, max_retries: 2 },
            }),
          }),
        ],
        {
          run: { ...runDetail([]).run, state: '{"current_nodes":["iter"]}' },
          failedAttempts: [
            persistedAttempt(fixRound0, { iteration: 0 }),
            persistedAttempt(fixRound1, { iteration: 1 }),
          ],
        },
      ),
      ITERATION_GRAPH,
    );

    const round1State = {
      iteration: 1,
      status: "retry_waiting",
      retryWait: {
        attempt: 2,
        maxAttempt: 3,
        retry: 1,
        maxRetries: 2,
        delayMs: 2_000,
        scheduledAt: BASE_MS + 35_000,
        dueAt: BASE_MS + 37_000,
      },
      autoRetry: { retry: 1, maxRetries: 2 },
      failedAttempts: [
        shownAttempt(fixRound1, {
          iteration: 1,
          replacedBy: "automatic_retry_scheduled",
        }),
      ],
    };
    expect(display.roundStates?.fix).toStrictEqual([
      {
        iteration: 0,
        status: "succeeded",
        sessionId: "session-fix-round-0-2",
        startedAt: iso(20_000),
        finishedAt: iso(25_000),
        autoRetry: { retry: 1, maxRetries: 2 },
        failedAttempts: [
          shownAttempt(fixRound0, {
            iteration: 0,
            replacedBy: "automatic_retry",
          }),
        ],
      },
      round1State,
    ]);
    expect(display.nodeStates.fix).toStrictEqual(round1State);
    expect(display.nodeStates.iter).not.toHaveProperty("failedAttempts");
  });

  it("puts Loop body attempts on their own Loop round only", () => {
    // Loop body rows have no round number, so attempt numbers continue across Loop rounds.
    const childRound0: AttemptFixture = {
      id: "child-attempt-1",
      nodeId: "child",
      attempt: 1,
      at: 10_000,
    };
    const childRound1: AttemptFixture = {
      id: "child-attempt-2",
      nodeId: "child",
      attempt: 2,
      at: 40_000,
    };
    const display = buildDisplayRun(
      runDetail(
        [
          START_ROW,
          nodeRow("loop-1", {
            id: "node-run-loop",
            status: "running",
            startedAt: ms(2_000),
          }),
          nodeRow("child", {
            id: "child-round-0",
            scopeId: "loop-scope-0",
            status: "succeeded",
            sessionId: "session-child-round-0",
            startedAt: ms(20_000),
            finishedAt: ms(30_000),
            payload: payload({ auto_retry: { retry: 1, max_retries: 3 } }),
          }),
          nodeRow("child", {
            id: "child-round-1",
            scopeId: "loop-scope-1",
            status: "running",
            payload: payload({
              retry_wait: {
                attempt: 3,
                max_attempt: 5,
                retry: 1,
                max_retries: 3,
                delay_ms: 1_000,
                scheduled_at: BASE_MS + 45_000,
                due_at: BASE_MS + 46_000,
                previous_node_run_id: "child-attempt-2",
              },
              auto_retry: { retry: 1, max_retries: 3 },
            }),
          }),
        ],
        {
          run: {
            ...runDetail([]).run,
            state: '{"current_nodes":["loop-1"]}',
          },
          scopes: [
            {
              id: "loop-scope-0",
              parentLoopNodeRunId: "node-run-loop",
              roundIndex: 0,
              status: "succeeded" as const,
              createdAt: ms(3_000),
              updatedAt: ms(30_000),
            },
            {
              id: "loop-scope-1",
              parentLoopNodeRunId: "node-run-loop",
              roundIndex: 1,
              status: "running" as const,
              createdAt: ms(31_000),
              updatedAt: ms(45_000),
            },
          ],
          failedAttempts: [
            persistedAttempt(childRound0, { scopeId: "loop-scope-0" }),
            persistedAttempt(childRound1, { scopeId: "loop-scope-1" }),
          ],
        },
      ),
      LOOP_GRAPH,
    );

    expect(display.rounds?.map((round) => round.nodeStates)).toStrictEqual([
      {
        child: {
          status: "succeeded",
          sessionId: "session-child-round-0",
          startedAt: iso(20_000),
          finishedAt: iso(30_000),
          autoRetry: { retry: 1, maxRetries: 3 },
          failedAttempts: [
            shownAttempt(childRound0, {
              loopRoundIndex: 0,
              replacedBy: "automatic_retry",
            }),
          ],
        },
      },
      {
        // Had round 0's attempt 1 leaked in, it would show here as a manual resume of attempt 2.
        child: {
          status: "retry_waiting",
          retryWait: {
            attempt: 3,
            maxAttempt: 5,
            retry: 1,
            maxRetries: 3,
            delayMs: 1_000,
            scheduledAt: BASE_MS + 45_000,
            dueAt: BASE_MS + 46_000,
          },
          autoRetry: { retry: 1, maxRetries: 3 },
          failedAttempts: [
            shownAttempt(childRound1, {
              loopRoundIndex: 1,
              replacedBy: "automatic_retry_scheduled",
            }),
          ],
        },
      },
    ]);
    // The Loop body node is not a root node, so its root state carries no attempts.
    expect(display.nodeStates.child).toStrictEqual({ status: "idle" });
    expect(display.nodeStates["loop-1"]).not.toHaveProperty("failedAttempts");
  });

  it("marks attempts from before a restart and does not read the restart as a resume", () => {
    const display = buildDisplayRun(
      runDetail(
        [
          nodeRow("start", {
            scopeId: "root:restarted",
            startedAt: ms(15_000),
            finishedAt: ms(16_000),
          }),
          nodeRow("explore", {
            scopeId: "root:restarted",
            status: "running",
            payload: payload({
              retry_wait: { ...RETRY_WAIT_ATTEMPT_3, retry: 1, max_attempt: 5 },
              auto_retry: { retry: 1, max_retries: 3 },
            }),
          }),
        ],
        {
          failedAttempts: [
            persistedAttempt(EXPLORE_1, { scopeId: ROOT_SCOPE }),
            persistedAttempt(EXPLORE_2, { scopeId: "root:restarted" }),
          ],
        },
      ),
      PARALLEL_GRAPH,
    );

    expect(display.nodeStates.explore.failedAttempts).toStrictEqual([
      shownAttempt(EXPLORE_1, { beforeRestart: true }),
      shownAttempt(EXPLORE_2, { replacedBy: "automatic_retry_scheduled" }),
    ]);
  });

  it("treats all attempts as one execution when rows carry no scope id", () => {
    const display = buildDisplayRun(
      runDetail(
        [
          START_ROW,
          nodeRow("explore", {
            scopeId: undefined,
            status: "failed",
            error: "agent session failed",
            payload: payload({
              error_detail: {
                kind: "session",
                message: "agent session failed",
                attempt: 3,
              },
            }),
          }),
        ],
        {
          failedAttempts: [
            persistedAttempt(EXPLORE_1, { scopeId: ROOT_SCOPE }),
            persistedAttempt(EXPLORE_2, { scopeId: "root:restarted" }),
          ],
        },
      ),
      PARALLEL_GRAPH,
    );

    expect(display.nodeStates.explore.failedAttempts).toStrictEqual([
      shownAttempt(EXPLORE_1),
      shownAttempt(EXPLORE_2, { replacedBy: "manual_resume" }),
    ]);
  });

  it("projects a detail from a backend that predates failedAttempts", () => {
    const detail = runDetail([
      START_ROW,
      nodeRow("explore", {
        status: "failed",
        startedAt: ms(34_000),
        finishedAt: ms(40_000),
        error: "agent session failed",
        payload: payload({ auto_retry: { retry: 3, max_retries: 3 } }),
      }),
    ]);
    expect(detail).not.toHaveProperty("failedAttempts");

    const display = buildDisplayRun(detail, PARALLEL_GRAPH);

    expect(display.nodeStates).toStrictEqual({
      start: {
        status: "succeeded",
        startedAt: iso(0),
        finishedAt: iso(1_000),
      },
      explore: {
        status: "failed",
        startedAt: iso(34_000),
        finishedAt: iso(40_000),
        errorMessage: "agent session failed",
        autoRetry: { retry: 3, maxRetries: 3 },
      },
      review: { status: "idle" },
    });
  });

  it("projects attempts from a backend that predates sourceChain with an empty chain", () => {
    const withoutChain = persistedAttempt(EXPLORE_1);
    delete withoutChain.sourceChain;
    const display = buildDisplayRun(
      runDetail(
        [
          START_ROW,
          nodeRow("explore", {
            status: "running",
            startedAt: ms(20_000),
            sessionId: "session-explore-2",
          }),
        ],
        { failedAttempts: [withoutChain] },
      ),
      PARALLEL_GRAPH,
    );

    expect(display.nodeStates.explore.failedAttempts).toStrictEqual([
      shownAttempt(EXPLORE_1, { sourceChain: [] }),
    ]);
  });
});
