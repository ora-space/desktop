import { describe, expect, it } from "vitest";
import type { WorkflowNodeAttemptFailure } from "@ora/workflow-runtime";
import {
  attemptHistoryKey,
  groupFailedAttempts,
  isRetryAbandoned,
  parseAutoRetry,
  parseRetryWait,
  projectFailedAttempts,
  recordedAttemptNumber,
  type AttemptHistoryOwner,
  type PersistedFailedAttempt,
} from "./workflow-run-retry";

const ROOT_SCOPE = "root:run-1";
const BASE_MS = 1_700_000_000_000;

/** Attempt `n` of `explore` in the live root scope; it ran from n*10s to n*10s+4s after BASE_MS. */
function failedAttempt(
  attempt: number,
  overrides: Partial<PersistedFailedAttempt> = {},
): PersistedFailedAttempt {
  const startMs = BASE_MS + attempt * 10_000;
  return {
    nodeRunId: `node-run-${attempt}`,
    nodeId: "explore",
    scopeId: ROOT_SCOPE,
    iteration: null,
    sessionId: `session-${attempt}`,
    attempt,
    kind: "session",
    message: `智能体会话失败（第 ${attempt} 次）`,
    sourceChain: ["agent session failed", `prompt ${attempt} timed out`],
    recordedAt: BigInt(startMs + 5_000),
    startedAt: BigInt(startMs),
    finishedAt: BigInt(startMs + 4_000),
    ...overrides,
  };
}

/** What `failedAttempt(attempt)` projects to before any marks are added. */
function projectedAttempt(
  attempt: number,
  extra: Partial<WorkflowNodeAttemptFailure> = {},
): WorkflowNodeAttemptFailure {
  const startMs = BASE_MS + attempt * 10_000;
  return {
    nodeRunId: `node-run-${attempt}`,
    attempt,
    kind: "session",
    errorMessage: `智能体会话失败（第 ${attempt} 次）`,
    sourceChain: ["agent session failed", `prompt ${attempt} timed out`],
    recordedAt: startMs + 5_000,
    startedAt: new Date(startMs).toISOString(),
    finishedAt: new Date(startMs + 4_000).toISOString(),
    sessionId: `session-${attempt}`,
    ...extra,
  };
}

/**
 * A started live row in the root scope that did not start from a retry and records no attempt
 * number.
 */
function owner(
  overrides: Partial<AttemptHistoryOwner> = {},
): AttemptHistoryOwner {
  return {
    scopeId: ROOT_SCOPE,
    autoRetry: undefined,
    attempt: undefined,
    started: true,
    ...overrides,
  };
}

describe("parseRetryWait", () => {
  const marker = {
    attempt: 3,
    max_attempt: 4,
    retry: 2,
    max_retries: 3,
    delay_ms: 4_000,
    scheduled_at: 1_700_000_030_000,
    due_at: 1_700_000_034_000,
    previous_node_run_id: "node-run-2",
  };

  it("maps all seven fields of a complete marker and ignores previous_node_run_id", () => {
    expect(parseRetryWait(marker)).toStrictEqual({
      attempt: 3,
      maxAttempt: 4,
      retry: 2,
      maxRetries: 3,
      delayMs: 4_000,
      scheduledAt: 1_700_000_030_000,
      dueAt: 1_700_000_034_000,
    });
  });

  it.each([
    "attempt",
    "max_attempt",
    "retry",
    "max_retries",
    "delay_ms",
    "scheduled_at",
    "due_at",
  ] as const)("returns undefined when %s is missing", (field) => {
    const partial: Record<string, unknown> = { ...marker };
    delete partial[field];
    expect(parseRetryWait(partial)).toBeUndefined();
  });

  it.each([
    ["a fractional number", 1.5],
    ["a numeric string", "3"],
    ["null", null],
    ["NaN", Number.NaN],
    ["Infinity", Number.POSITIVE_INFINITY],
  ])("returns undefined when a field is %s", (_label, value) => {
    expect(parseRetryWait({ ...marker, retry: value })).toBeUndefined();
    expect(parseRetryWait({ ...marker, due_at: value })).toBeUndefined();
  });

  it.each([
    ["null", null],
    ["undefined", undefined],
    ["an array", [3, 4, 2, 3, 4_000, 1, 2]],
    ["a string", JSON.stringify(marker)],
    ["a number", 3],
  ])("returns undefined when the marker is %s", (_label, value) => {
    expect(parseRetryWait(value)).toBeUndefined();
  });
});

describe("parseAutoRetry", () => {
  it("maps retry and max_retries and ignores other keys", () => {
    expect(
      parseAutoRetry({ retry: 1, max_retries: 3, previous: "x" }),
    ).toStrictEqual({ retry: 1, maxRetries: 3 });
  });

  it.each([
    ["retry is missing", { max_retries: 3 }],
    ["max_retries is missing", { retry: 1 }],
    ["retry is a string", { retry: "1", max_retries: 3 }],
    ["max_retries is fractional", { retry: 1, max_retries: 2.5 }],
    ["the marker is null", null],
    ["the marker is an array", [1, 3]],
    ["the marker is a string", '{"retry":1,"max_retries":3}'],
  ])("returns undefined when %s", (_label, value) => {
    expect(parseAutoRetry(value)).toBeUndefined();
  });
});

describe("recordedAttemptNumber", () => {
  it("reads the attempt that error_detail recorded", () => {
    expect(
      recordedAttemptNumber({ kind: "session", message: "失败", attempt: 4 }),
    ).toBe(4);
  });

  it.each([
    ["attempt is missing", { kind: "session", message: "失败" }],
    ["attempt is a string", { kind: "session", attempt: "4" }],
    ["attempt is fractional", { kind: "session", attempt: 2.5 }],
    ["error_detail is null", null],
    ["error_detail is undefined", undefined],
    ["error_detail is an array", [4]],
  ])(
    "returns undefined instead of defaulting to 1 when %s",
    (_label, value) => {
      expect(recordedAttemptNumber(value)).toBeUndefined();
    },
  );
});

describe("isRetryAbandoned", () => {
  it("is true for a cancelled row with the retry_abandoned reason", () => {
    expect(isRetryAbandoned("cancelled", '{"reason":"retry_abandoned"}')).toBe(
      true,
    );
  });

  it("parses the error as JSON instead of comparing strings", () => {
    expect(
      isRetryAbandoned("cancelled", '{ "reason": "retry_abandoned" }'),
    ).toBe(true);
  });

  it.each(["failed", "running", "succeeded", "pending"])(
    "is false for a %s row with the same error",
    (status) => {
      expect(isRetryAbandoned(status, '{"reason":"retry_abandoned"}')).toBe(
        false,
      );
    },
  );

  it.each([
    ["a null error (user cancel)", null],
    ["a restart interruption", '{"reason":"interrupted_by_restart"}'],
    ["non-JSON text", "retry_abandoned"],
    ["a JSON string", '"retry_abandoned"'],
    ["a JSON array", '["retry_abandoned"]'],
    ["an object without reason", '{"retry_abandoned":true}'],
  ])("is false for a cancelled row with %s", (_label, error) => {
    expect(isRetryAbandoned("cancelled", error)).toBe(false);
  });
});

describe("attemptHistoryKey", () => {
  it("keys root and region rows by node and round only", () => {
    expect(attemptHistoryKey("explore", null, null)).toBe(
      attemptHistoryKey("explore", undefined, null),
    );
    expect(attemptHistoryKey("fix", 0, null)).toBe(
      attemptHistoryKey("fix", 0, null),
    );
    expect(attemptHistoryKey("fix", 0, null)).not.toBe(
      attemptHistoryKey("fix", 1, null),
    );
    expect(attemptHistoryKey("fix", 0, null)).not.toBe(
      attemptHistoryKey("fix", null, null),
    );
    expect(attemptHistoryKey("explore", null, null)).not.toBe(
      attemptHistoryKey("review", null, null),
    );
  });

  it("keys Loop body rows by their Loop scope", () => {
    expect(attemptHistoryKey("child", null, "scope-1")).not.toBe(
      attemptHistoryKey("child", null, "scope-2"),
    );
    expect(attemptHistoryKey("child", null, "scope-1")).not.toBe(
      attemptHistoryKey("child", null, null),
    );
    // A scope id that looks like a round number must not collide with that round.
    expect(attemptHistoryKey("child", null, "0")).not.toBe(
      attemptHistoryKey("child", 0, null),
    );
  });
});

describe("groupFailedAttempts", () => {
  it("groups root attempts by node regardless of scope, so attempts before a restart stay with the node", () => {
    const beforeRestart = failedAttempt(1, { scopeId: "root:old" });
    const afterRestart = failedAttempt(2, { scopeId: "root:new" });
    const other = failedAttempt(1, {
      nodeRunId: "review-1",
      nodeId: "review",
      scopeId: "root:new",
    });

    const groups = groupFailedAttempts(
      [beforeRestart, other, afterRestart],
      new Set(),
    );

    expect([...groups.entries()]).toStrictEqual([
      [attemptHistoryKey("explore", null, null), [beforeRestart, afterRestart]],
      [attemptHistoryKey("review", null, null), [other]],
    ]);
  });

  it("splits region attempts per round and keeps each round oldest first", () => {
    const round0First = failedAttempt(1, { nodeRunId: "r0-1", iteration: 0 });
    const round1First = failedAttempt(1, { nodeRunId: "r1-1", iteration: 1 });
    const round0Second = failedAttempt(2, { nodeRunId: "r0-2", iteration: 0 });

    const groups = groupFailedAttempts(
      [round0First, round1First, round0Second],
      new Set(),
    );

    expect([...groups.entries()]).toStrictEqual([
      [attemptHistoryKey("explore", 0, null), [round0First, round0Second]],
      [attemptHistoryKey("explore", 1, null), [round1First]],
    ]);
  });

  it("splits Loop body attempts per Loop scope", () => {
    const scope1First = failedAttempt(1, { nodeRunId: "s1-1", scopeId: "s1" });
    const scope2 = failedAttempt(2, { nodeRunId: "s2-2", scopeId: "s2" });
    const scope1Second = failedAttempt(3, { nodeRunId: "s1-3", scopeId: "s1" });
    const root = failedAttempt(1, { nodeRunId: "root-1" });

    const groups = groupFailedAttempts(
      [scope1First, scope2, scope1Second, root],
      new Set(["s1", "s2"]),
    );

    expect([...groups.entries()]).toStrictEqual([
      [attemptHistoryKey("explore", null, "s1"), [scope1First, scope1Second]],
      [attemptHistoryKey("explore", null, "s2"), [scope2]],
      [attemptHistoryKey("explore", null, null), [root]],
    ]);
  });

  it("returns an empty map for no attempts", () => {
    expect(groupFailedAttempts([], new Set(["s1"])).size).toBe(0);
  });
});

describe("projectFailedAttempts", () => {
  it("returns nothing for no attempts", () => {
    expect(projectFailedAttempts([], owner(), undefined)).toStrictEqual([]);
  });

  it("marks the attempt before a waiting retry 2 as scheduled and the one before as retried", () => {
    // Attempt 2 was itself retry 1 and ran; retry 2 is still waiting, so it has not run.
    expect(
      projectFailedAttempts(
        [failedAttempt(1), failedAttempt(2)],
        owner({
          attempt: 3,
          autoRetry: { retry: 2, maxRetries: 3 },
          started: false,
        }),
        undefined,
      ),
    ).toStrictEqual([
      projectedAttempt(1, { replacedBy: "automatic_retry" }),
      projectedAttempt(2, { replacedBy: "automatic_retry_scheduled" }),
    ]);
  });

  it("marks the attempt before a waiting retry 1 as a scheduled retry, not a retry that ran", () => {
    expect(
      projectFailedAttempts(
        [failedAttempt(1)],
        owner({
          attempt: 2,
          autoRetry: { retry: 1, maxRetries: 3 },
          started: false,
        }),
        undefined,
      ),
    ).toStrictEqual([
      projectedAttempt(1, { replacedBy: "automatic_retry_scheduled" }),
    ]);
  });

  it("marks the attempt before a started retry 1 as an automatic retry", () => {
    expect(
      projectFailedAttempts(
        [failedAttempt(1)],
        owner({ attempt: 2, autoRetry: { retry: 1, maxRetries: 3 } }),
        undefined,
      ),
    ).toStrictEqual([projectedAttempt(1, { replacedBy: "automatic_retry" })]);
  });

  it("uses the listed attempt's own start to tell whether an older retry ran", () => {
    // Each mark reads the start of the row that replaced the attempt: attempt 3 started, so
    // retry 2 ran; attempt 2 has no start time, so retry 1 never ran.
    expect(
      projectFailedAttempts(
        [failedAttempt(1), failedAttempt(2, { startedAt: null })],
        owner({ attempt: 3, autoRetry: { retry: 2, maxRetries: 3 } }),
        undefined,
      ),
    ).toStrictEqual([
      projectedAttempt(1, { replacedBy: "automatic_retry_scheduled" }),
      (() => {
        const projected = projectedAttempt(2, {
          replacedBy: "automatic_retry",
        });
        delete projected.startedAt;
        return projected;
      })(),
    ]);
  });

  it("marks only the directly preceding attempt of a fresh failed row as a manual resume", () => {
    expect(
      projectFailedAttempts(
        [failedAttempt(1), failedAttempt(2), failedAttempt(3)],
        owner({ attempt: 4 }),
        undefined,
      ),
    ).toStrictEqual([
      projectedAttempt(1),
      projectedAttempt(2),
      projectedAttempt(3, { replacedBy: "manual_resume" }),
    ]);
  });

  it("follows an automatic retry back to the manual resume that started its chain", () => {
    // Attempt 2 started fresh (a manual resume of 1), failed, and was retried as attempt 3.
    expect(
      projectFailedAttempts(
        [failedAttempt(1), failedAttempt(2)],
        owner({ attempt: 3, autoRetry: { retry: 1, maxRetries: 3 } }),
        undefined,
      ),
    ).toStrictEqual([
      projectedAttempt(1, { replacedBy: "manual_resume" }),
      projectedAttempt(2, { replacedBy: "automatic_retry" }),
    ]);
  });

  it("stops the automatic chain once its retries are used up and leaves older attempts unmarked", () => {
    // Attempt 3 started fresh after 2, so 2 was resumed; nothing proves how 1 was replaced.
    expect(
      projectFailedAttempts(
        [failedAttempt(1), failedAttempt(2), failedAttempt(3)],
        owner({ attempt: 4, autoRetry: { retry: 1, maxRetries: 3 } }),
        undefined,
      ),
    ).toStrictEqual([
      projectedAttempt(1),
      projectedAttempt(2, { replacedBy: "manual_resume" }),
      projectedAttempt(3, { replacedBy: "automatic_retry" }),
    ]);
  });

  it("leaves an attempt unmarked when a hidden row sits between it and a fresh owner", () => {
    // Attempt 2 was cancelled (not listed), so attempt 3 did not replace attempt 1.
    expect(
      projectFailedAttempts(
        [failedAttempt(1)],
        owner({ attempt: 3 }),
        undefined,
      ),
    ).toStrictEqual([projectedAttempt(1)]);
  });

  it("leaves an attempt unmarked when a hidden row sits between it and a retry", () => {
    expect(
      projectFailedAttempts(
        [failedAttempt(1)],
        owner({ attempt: 3, autoRetry: { retry: 1, maxRetries: 3 } }),
        undefined,
      ),
    ).toStrictEqual([projectedAttempt(1)]);
  });

  it("marks nothing for a running row with no recorded attempt and no retry marker", () => {
    expect(
      projectFailedAttempts(
        [failedAttempt(1), failedAttempt(2)],
        owner(),
        undefined,
      ),
    ).toStrictEqual([projectedAttempt(1), projectedAttempt(2)]);
  });

  it("marks automatic retries of a running row whose attempt number is unknown", () => {
    expect(
      projectFailedAttempts(
        [failedAttempt(1)],
        owner({ autoRetry: { retry: 1, maxRetries: 3 } }),
        undefined,
      ),
    ).toStrictEqual([projectedAttempt(1, { replacedBy: "automatic_retry" })]);
    expect(
      projectFailedAttempts(
        [failedAttempt(1), failedAttempt(2)],
        owner({ autoRetry: { retry: 2, maxRetries: 3 } }),
        undefined,
      ),
    ).toStrictEqual([
      projectedAttempt(1, { replacedBy: "automatic_retry" }),
      projectedAttempt(2, { replacedBy: "automatic_retry" }),
    ]);
  });

  it("marks attempts of another scope as before a restart and never as replaced", () => {
    // Attempt 1 failed before "run again from start"; attempt 2 failed after it and is retried.
    // Without the scope check, attempt 1 would look like a manual resume of attempt 2's chain.
    expect(
      projectFailedAttempts(
        [
          failedAttempt(1, { scopeId: "root:old" }),
          failedAttempt(2, { scopeId: "root:new" }),
        ],
        owner({
          scopeId: "root:new",
          attempt: 3,
          autoRetry: { retry: 1, maxRetries: 3 },
        }),
        undefined,
      ),
    ).toStrictEqual([
      projectedAttempt(1, { beforeRestart: true }),
      projectedAttempt(2, { replacedBy: "automatic_retry" }),
    ]);
  });

  it("does not read a restart as a manual resume of the attempt before it", () => {
    expect(
      projectFailedAttempts(
        [failedAttempt(1, { scopeId: "root:old" })],
        owner({ scopeId: "root:new", attempt: 2 }),
        undefined,
      ),
    ).toStrictEqual([projectedAttempt(1, { beforeRestart: true })]);
  });

  it("treats every attempt as the live execution when the owner's scope is unknown", () => {
    expect(
      projectFailedAttempts(
        [
          failedAttempt(1, { scopeId: "root:old" }),
          failedAttempt(2, { scopeId: "root:new" }),
        ],
        owner({ scopeId: undefined, attempt: 3 }),
        undefined,
      ),
    ).toStrictEqual([
      projectedAttempt(1),
      projectedAttempt(2, { replacedBy: "manual_resume" }),
    ]);
  });

  it("projects bigint times and copies the region round, including round 0", () => {
    expect(
      projectFailedAttempts(
        [
          {
            nodeRunId: "fix-round-0-attempt-1",
            nodeId: "fix",
            scopeId: ROOT_SCOPE,
            iteration: 0,
            sessionId: "session-fix-1",
            attempt: 1,
            kind: "structured_output",
            message: "输出不是合法 JSON",
            sourceChain: ["structured output invalid", "expected value"],
            recordedAt: 1_700_000_005_000n,
            startedAt: 1_700_000_000_000n,
            finishedAt: 1_700_000_004_000n,
          },
        ],
        owner(),
        undefined,
      ),
    ).toStrictEqual([
      {
        nodeRunId: "fix-round-0-attempt-1",
        attempt: 1,
        kind: "structured_output",
        errorMessage: "输出不是合法 JSON",
        sourceChain: ["structured output invalid", "expected value"],
        recordedAt: 1_700_000_005_000,
        startedAt: "2023-11-14T22:13:20.000Z",
        finishedAt: "2023-11-14T22:13:24.000Z",
        sessionId: "session-fix-1",
        iteration: 0,
      },
    ]);
  });

  it("omits a missing session, round and times, and defaults a missing sourceChain to []", () => {
    expect(
      projectFailedAttempts(
        [
          {
            nodeRunId: "old-backend-1",
            nodeId: "explore",
            scopeId: ROOT_SCOPE,
            iteration: null,
            sessionId: null,
            attempt: 1,
            kind: "timeout",
            message: "节点超时",
            recordedAt: 1_700_000_005_000n,
            startedAt: null,
            finishedAt: null,
          },
          {
            nodeRunId: "old-backend-2",
            nodeId: "explore",
            scopeId: ROOT_SCOPE,
            iteration: null,
            sessionId: "",
            attempt: 2,
            kind: "timeout",
            message: "节点超时",
            recordedAt: 1_700_000_015_000n,
            startedAt: 1_700_000_010_000n,
            finishedAt: null,
          },
        ],
        owner(),
        undefined,
      ),
    ).toStrictEqual([
      {
        nodeRunId: "old-backend-1",
        attempt: 1,
        kind: "timeout",
        errorMessage: "节点超时",
        sourceChain: [],
        recordedAt: 1_700_000_005_000,
      },
      {
        nodeRunId: "old-backend-2",
        attempt: 2,
        kind: "timeout",
        errorMessage: "节点超时",
        sourceChain: [],
        recordedAt: 1_700_000_015_000,
        startedAt: "2023-11-14T22:13:30.000Z",
      },
    ]);
  });

  it("copies the Loop round index onto every attempt, including round 0", () => {
    const inScope = (attempt: number) =>
      failedAttempt(attempt, { scopeId: "loop-scope-1" });
    expect(
      projectFailedAttempts(
        [inScope(1), inScope(2)],
        owner({
          scopeId: "loop-scope-1",
          attempt: 3,
          autoRetry: { retry: 1, maxRetries: 2 },
        }),
        0,
      ),
    ).toStrictEqual([
      projectedAttempt(1, { loopRoundIndex: 0, replacedBy: "manual_resume" }),
      projectedAttempt(2, {
        loopRoundIndex: 0,
        replacedBy: "automatic_retry",
      }),
    ]);
    expect(
      projectFailedAttempts(
        [inScope(1)],
        owner({ scopeId: "loop-scope-1" }),
        2,
      ),
    ).toStrictEqual([projectedAttempt(1, { loopRoundIndex: 2 })]);
  });
});
