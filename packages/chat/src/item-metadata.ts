import type { ChatItemMetadata, ChatToolCall } from "./types.ts";

function object(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

/** Canonicalizes JSON objects so wire key order cannot change live/replay merge boundaries. */
function canonical(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonical);
  if (object(value))
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .filter((key) => value[key] !== undefined)
        .map((key) => [key, canonical(value[key])]),
    );
  return value;
}

/** Full metadata and text annotations define a merge boundary, not merely a child id. */
export function metadataKey(
  metadata: unknown,
  context?: ChatItemMetadata["contentContext"],
): string | undefined {
  if (
    !object(metadata) &&
    context?.metadata === undefined &&
    context?.annotations === undefined
  )
    return undefined;
  return JSON.stringify(
    canonical([
      metadata ?? null,
      context?.metadata ?? null,
      context?.annotations ?? null,
    ]),
  );
}

const OPENCODE_CHILD_SESSION_KEY = "opencode/child-session";
const SUBAGENT_UPDATE_KEY = "_subagent/update";
const TURN_END_SETTLED_KEY = "ora/turn-end-settled";

export interface ChildSessionSource {
  id: string;
  parentId: string;
  depth: number;
  title?: string;
  /** Parent delegation tool call id explicitly reported by `_subagent/update`. */
  delegationToolCallId?: string;
}

/** Reports whether `metadata` carries any recognized child-projection namespace, even when cleared. */
export function hasChildSessionNamespace(metadata: unknown): boolean {
  return (
    object(metadata) &&
    (OPENCODE_CHILD_SESSION_KEY in metadata || SUBAGENT_UPDATE_KEY in metadata)
  );
}

function parseOpenCodeChildSession(
  child: unknown,
): ChildSessionSource | undefined {
  if (
    !object(child) ||
    typeof child.id !== "string" ||
    !child.id.trim() ||
    typeof child.parentID !== "string" ||
    !child.parentID.trim() ||
    typeof child.depth !== "number" ||
    !Number.isSafeInteger(child.depth) ||
    child.depth < 1
  )
    return undefined;
  return {
    id: child.id,
    parentId: child.parentID,
    depth: child.depth,
    ...(typeof child.title === "string" && child.title.trim()
      ? { title: child.title }
      : {}),
  };
}

function parseSubagentUpdateSource(
  subagent: unknown,
): ChildSessionSource | undefined {
  if (
    !object(subagent) ||
    typeof subagent.taskId !== "string" ||
    !subagent.taskId.trim() ||
    typeof subagent.sessionId !== "string" ||
    !subagent.sessionId.trim() ||
    (subagent.toolCallId !== undefined &&
      subagent.toolCallId !== null &&
      typeof subagent.toolCallId !== "string") ||
    (subagent.agentType !== undefined &&
      subagent.agentType !== null &&
      typeof subagent.agentType !== "string")
  )
    return undefined;
  return {
    id: subagent.taskId,
    parentId: subagent.sessionId,
    depth: 1,
    ...(typeof subagent.agentType === "string" && subagent.agentType.trim()
      ? { title: subagent.agentType }
      : {}),
    ...(typeof subagent.toolCallId === "string" && subagent.toolCallId.trim()
      ? { delegationToolCallId: subagent.toolCallId }
      : {}),
  };
}

/** Parses explicit OpenCode or `_subagent/update` attribution; names and prefixes never infer edges. */
export function childSessionSource(
  metadata: unknown,
): ChildSessionSource | undefined {
  if (!object(metadata)) return undefined;
  const hasOpenCode = OPENCODE_CHILD_SESSION_KEY in metadata;
  const hasSubagent = SUBAGENT_UPDATE_KEY in metadata;
  if (hasOpenCode && !hasSubagent)
    return parseOpenCodeChildSession(metadata[OPENCODE_CHILD_SESSION_KEY]);
  if (!hasOpenCode && hasSubagent)
    return parseSubagentUpdateSource(metadata[SUBAGENT_UPDATE_KEY]);
  if (hasOpenCode && hasSubagent) {
    const openCode = parseOpenCodeChildSession(
      metadata[OPENCODE_CHILD_SESSION_KEY],
    );
    const subagent = parseSubagentUpdateSource(metadata[SUBAGENT_UPDATE_KEY]);
    return openCode && subagent && openCode.id === subagent.id
      ? openCode
      : undefined;
  }
  return undefined;
}

/** Retains metadata without interpreting or recursively merging unknown provider values. */
export function withItemMetadata<Item extends ChatItemMetadata>(
  item: Item,
  metadata: unknown,
  contentContext?: ChatItemMetadata["contentContext"],
): Item {
  return {
    ...item,
    ...(object(metadata) ? { metadata } : {}),
    ...(contentContext?.metadata !== undefined ||
    contentContext?.annotations !== undefined
      ? { contentContext }
      : {}),
  };
}

export function mergeToolMetadata(
  previous: Record<string, unknown> | undefined,
  incoming: unknown,
): Record<string, unknown> | undefined {
  return object(incoming) ? { ...previous, ...incoming } : previous;
}

/** Strips Ora's internal turn-end settlement marker and reports it only for replayed history. */
export function extractToolMetadata(
  metadata: unknown,
  trustTurnEndSettled: boolean,
): {
  metadata: Record<string, unknown> | undefined;
  settledByTurnEnd: boolean;
} {
  if (!object(metadata)) {
    return { metadata: undefined, settledByTurnEnd: false };
  }
  if (!(TURN_END_SETTLED_KEY in metadata)) {
    return { metadata, settledByTurnEnd: false };
  }
  const settledByTurnEnd =
    trustTurnEndSettled && metadata[TURN_END_SETTLED_KEY] === true;
  const rest = Object.fromEntries(
    Object.entries(metadata).filter(([key]) => key !== TURN_END_SETTLED_KEY),
  );
  return {
    metadata: Object.keys(rest).length > 0 ? rest : undefined,
    settledByTurnEnd,
  };
}

export function withToolMetadata(
  tool: ChatToolCall,
  metadata: unknown,
): ChatToolCall {
  return withItemMetadata(tool, metadata);
}

/**
 * Reads only the provider's delegation link from a tool result.
 *
 * OpenCode names the delegated child in `rawOutput.metadata.sessionID`; titles, tool names,
 * result text and timing describe nothing about the relationship and stay out of this parse.
 */
export function delegationSessionId(tool: ChatToolCall): string | undefined {
  if (!object(tool.rawOutput)) return undefined;
  const metadata = tool.rawOutput.metadata;
  if (!object(metadata)) return undefined;
  const id = metadata.sessionID;
  return typeof id === "string" && id.trim().length > 0 ? id : undefined;
}

/**
 * Reports whether the provider's own result says this delegation returned a finished run.
 *
 * A parent tool the turn ended on is settled as `completed` by the chat store, so the tool
 * status alone cannot prove the provider reported it. The provider writes its `status`
 * beside the session id and only pairs `"completed"` with an ACP `completed` call when the
 * foreground delegation actually returned.
 */
export function delegationResultCompleted(tool: ChatToolCall): boolean {
  if (tool.status !== "completed" || !object(tool.rawOutput)) return false;
  const metadata = tool.rawOutput.metadata;
  return object(metadata) && metadata.status === "completed";
}

/**
 * Reports whether a parent tool call linked by `_subagent/update.toolCallId` returned a result.
 *
 * Turn-end settlement marks unfinished parent tools as `completed` with no output, so only a
 * completed call carrying provider output (and not an explicit background status) qualifies.
 */
export function delegationToolCallCompleted(tool: ChatToolCall): boolean {
  if (tool.status !== "completed" || tool.settledByTurnEnd) return false;
  if (tool.content.length === 0 && tool.rawOutput === undefined) return false;
  if (
    object(tool.rawOutput) &&
    object(tool.rawOutput.metadata) &&
    "status" in tool.rawOutput.metadata
  ) {
    return tool.rawOutput.metadata.status === "completed";
  }
  return true;
}
