import type {
  WorkflowNodeAttemptFailure,
  WorkflowNodeAttemptReplacement,
  WorkflowNodeAutoRetry,
  WorkflowNodeRetryWait,
} from "@ora/workflow-runtime";

/**
 * One `failedAttempts` entry of `get_workflow_run`. Kept structural (and `sourceChain` optional)
 * so the adapter also accepts details from a backend that predates the field.
 */
export type PersistedFailedAttempt = {
  nodeRunId: string;
  nodeId: string;
  scopeId: string;
  iteration: number | null;
  sessionId: string | null;
  attempt: number;
  kind: string;
  message: string;
  sourceChain?: string[];
  recordedAt: bigint;
  startedAt: bigint | null;
  finishedAt: bigint | null;
};

/** The live row an attempt history belongs to, as far as the replacement rules need it. */
export type AttemptHistoryOwner = {
  scopeId: string | undefined;
  /** `payload.auto_retry` of the live row; absent means it did not start from a retry. */
  autoRetry: WorkflowNodeAutoRetry | undefined;
  /** The live row's attempt number when the payload records it (waiting or failed rows). */
  attempt: number | undefined;
  /**
   * Whether the live row ever started. A row scheduled by an automatic retry that is still
   * waiting, or whose wait was cancelled, abandoned, or failed by an app restart, never did.
   */
  started: boolean;
};

/** Error the backend writes on a waiting retry that the run gave up on when it ended. */
const RETRY_ABANDONED_REASON = "retry_abandoned";

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object" && !Array.isArray(value);
}

function isInteger(value: unknown): value is number {
  return typeof value === "number" && Number.isInteger(value);
}

/**
 * Reads `payload.retry_wait`. The backend always writes the whole struct, so a partial or
 * non-numeric marker is treated as absent rather than rendered with invented numbers.
 */
export function parseRetryWait(
  value: unknown,
): WorkflowNodeRetryWait | undefined {
  if (!isRecord(value)) {
    return undefined;
  }
  const fields = [
    value.attempt,
    value.max_attempt,
    value.retry,
    value.max_retries,
    value.delay_ms,
    value.scheduled_at,
    value.due_at,
  ];
  if (!fields.every(isInteger)) {
    return undefined;
  }
  const [attempt, maxAttempt, retry, maxRetries, delayMs, scheduledAt, dueAt] =
    fields as number[];
  return {
    attempt: attempt!,
    maxAttempt: maxAttempt!,
    retry: retry!,
    maxRetries: maxRetries!,
    delayMs: delayMs!,
    scheduledAt: scheduledAt!,
    dueAt: dueAt!,
  };
}

/** Reads `payload.auto_retry`, the retry that started an attempt. */
export function parseAutoRetry(
  value: unknown,
): WorkflowNodeAutoRetry | undefined {
  if (
    !isRecord(value) ||
    !isInteger(value.retry) ||
    !isInteger(value.max_retries)
  ) {
    return undefined;
  }
  return { retry: value.retry, maxRetries: value.max_retries };
}

/**
 * Reads `error_detail.attempt` only when the payload recorded it; the display projection defaults
 * a missing number to 1, which would be a guess here.
 */
export function recordedAttemptNumber(
  errorDetail: unknown,
): number | undefined {
  return isRecord(errorDetail) && isInteger(errorDetail.attempt)
    ? errorDetail.attempt
    : undefined;
}

/** True for a cancelled row whose error is the backend's `retry_abandoned` reason. */
export function isRetryAbandoned(status: string, error: string | null) {
  if (status !== "cancelled" || error == null) {
    return false;
  }
  try {
    const parsed: unknown = JSON.parse(error);
    return isRecord(parsed) && parsed.reason === RETRY_ABANDONED_REASON;
  } catch {
    return false;
  }
}

/**
 * Keys one node execution's attempts. Outer and iteration-region rows share the root scope and
 * are told apart by round; a restart gives the root scope a new id, so scope is not part of their
 * key (the backend lists attempts replaced by a restart on purpose). Loop body rows have no round
 * number, so their Loop round scope is the key.
 */
export function attemptHistoryKey(
  nodeId: string,
  iteration: number | null | undefined,
  loopScopeId: string | null,
): string {
  return loopScopeId !== null
    ? `${nodeId}\u0000scope\u0000${loopScopeId}`
    : `${nodeId}\u0000round\u0000${iteration ?? ""}`;
}

/** Groups the run's failed attempts by node execution, keeping the backend's oldest-first order. */
export function groupFailedAttempts(
  attempts: readonly PersistedFailedAttempt[],
  loopScopeIds: ReadonlySet<string>,
): Map<string, PersistedFailedAttempt[]> {
  const groups = new Map<string, PersistedFailedAttempt[]>();
  for (const attempt of attempts) {
    const key = attemptHistoryKey(
      attempt.nodeId,
      attempt.iteration,
      loopScopeIds.has(attempt.scopeId) ? attempt.scopeId : null,
    );
    const group = groups.get(key) ?? [];
    group.push(attempt);
    groups.set(key, group);
  }
  return groups;
}

/**
 * Walks back from the live row and marks what replaced each failed attempt, as far as the run
 * detail proves it. `failedAttempts` lists only soft-deleted `Failed` rows; a cancelled or
 * abandoned wait, or a region row cleared by a composite resume, is soft-deleted without being
 * listed. Attempt numbers count every soft-deleted row, so a gap in the numbers reveals such a
 * hidden row.
 *
 * - A row scheduled by automatic retry `k` (`auto_retry.retry`) was inserted in the transaction
 *   that soft-deleted the attempt before it, so that attempt was replaced by an automatic retry, and
 *   it was itself scheduled by retry `k - 1` (0 = a fresh start). The retry only ran if the row
 *   that carries it started; otherwise the attempt is marked as having a retry scheduled that
 *   never started, so the view never claims a retry that did not run.
 * - A fresh row whose attempt number directly follows a failed attempt in the same scope replaced
 *   it through a manual resume: automatic retries always mark their row, and a restart opens a
 *   new root scope.
 * - Anything older, or behind a gap, stays unmarked: the listed attempts do not carry their own
 *   `auto_retry` markers.
 */
function markReplacements(
  current: readonly PersistedFailedAttempt[],
  owner: AttemptHistoryOwner,
): Map<PersistedFailedAttempt, WorkflowNodeAttemptReplacement> {
  const marks = new Map<
    PersistedFailedAttempt,
    WorkflowNodeAttemptReplacement
  >();
  let next: {
    attempt: number | undefined;
    retry: number;
    started: boolean;
  } | null = {
    attempt: owner.attempt,
    retry: owner.autoRetry?.retry ?? 0,
    started: owner.started,
  };
  for (let index = current.length - 1; index >= 0 && next !== null; index--) {
    const attempt = current[index]!;
    const adjacent =
      next.attempt === undefined ? null : next.attempt === attempt.attempt + 1;
    if (next.retry >= 1 && adjacent !== false) {
      marks.set(
        attempt,
        next.started ? "automatic_retry" : "automatic_retry_scheduled",
      );
      next = {
        attempt: attempt.attempt,
        retry: next.retry - 1,
        started: attempt.startedAt != null,
      };
    } else if (next.retry === 0 && adjacent === true) {
      marks.set(attempt, "manual_resume");
      next = null;
    } else {
      next = null;
    }
  }
  return marks;
}

/**
 * Projects one node execution's failed attempts, oldest first. Only "run again from start" gives
 * the root scope a new id, so an attempt in another scope than the live row ran before a restart;
 * what replaced it is unknown (it may have been resumed first and the run restarted later).
 */
export function projectFailedAttempts(
  attempts: readonly PersistedFailedAttempt[],
  owner: AttemptHistoryOwner,
  loopRoundIndex: number | undefined,
): WorkflowNodeAttemptFailure[] {
  // Fixtures and older adapters may omit scope ids; everything then counts as one execution.
  const inLiveExecution = (attempt: PersistedFailedAttempt) =>
    owner.scopeId === undefined || attempt.scopeId === owner.scopeId;
  const marks = markReplacements(attempts.filter(inLiveExecution), owner);
  return attempts.map((attempt) => {
    const liveExecution = inLiveExecution(attempt);
    const replacedBy = marks.get(attempt);
    return {
      nodeRunId: attempt.nodeRunId,
      attempt: attempt.attempt,
      kind: attempt.kind,
      errorMessage: attempt.message,
      sourceChain: attempt.sourceChain ?? [],
      recordedAt: Number(attempt.recordedAt),
      ...(attempt.startedAt != null
        ? { startedAt: toIso(attempt.startedAt) }
        : {}),
      ...(attempt.finishedAt != null
        ? { finishedAt: toIso(attempt.finishedAt) }
        : {}),
      ...(attempt.sessionId != null && attempt.sessionId !== ""
        ? { sessionId: attempt.sessionId }
        : {}),
      ...(attempt.iteration != null ? { iteration: attempt.iteration } : {}),
      ...(loopRoundIndex !== undefined ? { loopRoundIndex } : {}),
      ...(replacedBy !== undefined ? { replacedBy } : {}),
      ...(liveExecution ? {} : { beforeRestart: true }),
    };
  });
}

function toIso(millis: bigint): string {
  return new Date(Number(millis)).toISOString();
}
