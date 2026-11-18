import {
  childSessionSource,
  delegationResultCompleted,
  delegationSessionId,
  delegationToolCallCompleted,
  type ChatToolCall,
  type ChatTurnItem,
} from "@ora/chat";

/** One explicitly reported source with a cycle-safe nested view of the same timeline. */
export interface SubagentGroupItem {
  kind: "subagentGroup";
  id: string;
  sourceId: string;
  parentId: string;
  depth: number;
  index: number;
  title?: string;
  delegationToolCallId?: string;
  relationshipUnknown: boolean;
  items: ChatTurnItem[];
  children: SubagentGroupItem[];
  timeline: (ChatTurnItem | SubagentGroupItem)[];
}

/** Groups explicit ids only; incomplete/conflicting relationships remain readable flat groups. */
export function groupSubagentItems(
  items: ChatTurnItem[],
): (ChatTurnItem | SubagentGroupItem)[] {
  const groups = new Map<string, SubagentGroupItem>();
  for (const item of items) {
    const source = childSessionSource(item.metadata);
    if (!source) continue;
    let group = groups.get(source.id);
    if (!group) {
      group = {
        kind: "subagentGroup",
        id: `subagent:${source.id}`,
        sourceId: source.id,
        parentId: source.parentId,
        depth: source.depth,
        index: groups.size + 1,
        title: source.title,
        delegationToolCallId: source.delegationToolCallId,
        relationshipUnknown: false,
        items: [],
        children: [],
        timeline: [],
      };
      groups.set(source.id, group);
    } else {
      if (group.parentId !== source.parentId || group.depth !== source.depth) {
        group.relationshipUnknown = true;
      }
      if (source.delegationToolCallId !== undefined) {
        if (group.delegationToolCallId === undefined) {
          group.delegationToolCallId = source.delegationToolCallId;
        } else if (group.delegationToolCallId !== source.delegationToolCallId) {
          group.relationshipUnknown = true;
        }
      }
    }
    group.title = source.title ?? group.title;
    group.items.push(item);
  }
  const parents = new Map<string, SubagentGroupItem>();
  for (const group of groups.values()) {
    const seen = new Set([group.sourceId]);
    let ancestor = groups.get(group.parentId);
    while (ancestor) {
      if (
        seen.has(ancestor.sourceId) ||
        seen.size >= 32 ||
        ancestor.relationshipUnknown
      ) {
        group.relationshipUnknown = true;
        break;
      }
      seen.add(ancestor.sourceId);
      ancestor = groups.get(ancestor.parentId);
    }
    const parent = groups.get(group.parentId);
    if (!parent && group.depth > 1) group.relationshipUnknown = true;
    if (parent && !group.relationshipUnknown) {
      parents.set(group.sourceId, parent);
      parent.children.push(group);
    }
  }
  const result: (ChatTurnItem | SubagentGroupItem)[] = [];
  const emitted = new Set<string>();
  for (const item of items) {
    const source = childSessionSource(item.metadata);
    if (!source) {
      result.push(item);
      continue;
    }
    let group = groups.get(source.id)!;
    group.timeline.push(item);
    while (parents.has(group.sourceId)) {
      const parent = parents.get(group.sourceId)!;
      if (!parent.timeline.includes(group)) parent.timeline.push(group);
      group = parent;
    }
    if (!emitted.has(group.sourceId)) {
      result.push(group);
      emitted.add(group.sourceId);
    }
  }
  return result;
}

/**
 * Reports whether the latest delegation to this child in one source stream returned a
 * finished result.
 *
 * Positions are the persisted ones: the latest call is the last match in stream order, not
 * the newest timestamp, and a later turn continuing the same child session never revises
 * an earlier turn. A child without an associable delegation keeps the projection-stopped
 * wording, because the absence of a link is not evidence of a running worker.
 */
export function delegationReturned(
  streamItems: ChatTurnItem[],
  childId: string,
  delegationToolCallId?: string,
): boolean {
  let latest: ChatToolCall | undefined;
  let matchedBySessionId = false;
  for (const item of streamItems) {
    if (item.kind !== "toolCall") continue;
    if (delegationSessionId(item) === childId) {
      latest = item;
      matchedBySessionId = true;
    } else if (
      delegationToolCallId !== undefined &&
      item.id === delegationToolCallId
    ) {
      latest = item;
      matchedBySessionId = false;
    }
  }
  if (latest === undefined) return false;
  return matchedBySessionId
    ? delegationResultCompleted(latest)
    : delegationToolCallCompleted(latest);
}

/**
 * Filters a turn to the parent session's own stream.
 *
 * Top-level children are launched from the parent stream, so a delegation found inside another
 * child's stream must not settle them; a nested group uses its parent group's items instead.
 */
export function parentStreamItems(items: ChatTurnItem[]): ChatTurnItem[] {
  return items.filter(
    (item) => childSessionSource(item.metadata) === undefined,
  );
}
