import type * as acp from "@agentclientprotocol/sdk";
import {
  childSessionSource,
  hasChildSessionNamespace,
} from "./item-metadata.ts";

/** A live observation, never reconstructed from transcript load time or provider health guesses. */
interface ObservedRunActivity {
  kind: "message" | "thought" | "tool" | "plan";
  receivedAt: number;
}

export interface RunDiagnostics {
  /** Parent-stream observations only; projected child output cannot reset this clock. */
  lastActivity?: ObservedRunActivity;
  lastChildActivity?: ObservedRunActivity;
  /** Live ownership evidence survives metadata-free deltas, matching runtime liveness. */
  toolSources?: Record<string, "parent" | "child">;
  provider?:
    | { kind: "retry"; attempt: number; nextRetryAt?: string }
    | { kind: "compaction"; status: "started" | "completed" | "failed" };
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Applies only recognized provider reports; unrecognized metadata never becomes a health claim. */
export function observeDiagnostics(
  previous: RunDiagnostics | undefined,
  update: acp.SessionUpdate,
  receivedAt: number,
): RunDiagnostics | undefined {
  if (
    (update.sessionUpdate === "agent_message_chunk" ||
      update.sessionUpdate === "agent_thought_chunk") &&
    update.content.type === "text" &&
    update.content.text === ""
  )
    return previous;
  const kinds = {
    agent_message_chunk: "message",
    agent_thought_chunk: "thought",
    tool_call: "tool",
    tool_call_update: "tool",
    plan: "plan",
  } as const;
  if (update.sessionUpdate in kinds) {
    const kind = kinds[update.sessionUpdate as keyof typeof kinds];
    let child = childSessionSource(update._meta) !== undefined;
    let toolSources = previous?.toolSources;
    if (
      update.sessionUpdate === "tool_call" ||
      update.sessionUpdate === "tool_call_update"
    ) {
      // Only an omitted namespace inherits; explicit null/invalid values reset ownership.
      if (!hasChildSessionNamespace(update._meta))
        child = toolSources?.[update.toolCallId] === "child";
      toolSources = {
        ...toolSources,
        [update.toolCallId]: child ? "child" : "parent",
      };
    }
    return {
      ...previous,
      ...(toolSources ? { toolSources } : {}),
      ...(child
        ? { lastChildActivity: { kind, receivedAt } }
        : { lastActivity: { kind, receivedAt } }),
    };
  }
  if (update.sessionUpdate !== "session_info_update" || !record(update._meta))
    return previous;
  if (childSessionSource(update._meta)) return previous;
  const retry = update._meta["opencode/retry"];
  if (
    record(retry) &&
    typeof retry.attempt === "number" &&
    Number.isSafeInteger(retry.attempt) &&
    retry.attempt > 0
  ) {
    return {
      ...previous,
      provider: {
        kind: "retry",
        attempt: retry.attempt,
        ...(typeof retry.nextRetryAt === "string"
          ? { nextRetryAt: retry.nextRetryAt }
          : {}),
      },
    };
  }
  const compaction = update._meta["opencode/compaction"];
  if (
    record(compaction) &&
    ["started", "completed", "failed"].includes(String(compaction.status))
  ) {
    return {
      ...previous,
      provider: {
        kind: "compaction",
        status: compaction.status as "started" | "completed" | "failed",
      },
    };
  }
  if (retry === null && previous?.provider?.kind === "retry")
    return { ...previous, provider: undefined };
  return previous;
}
