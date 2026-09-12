import type { ChatModelChange, ChatTurn } from "@ora/chat";
import { buildTurnDisplayItems } from "./turn-item-grouping";

/** Below this row count the thread stays fully mounted; jsdom tests never measure a viewport. */
export const MESSAGE_LIST_VIRTUALIZE_MIN_ROWS = 8;

export type MessageListRow =
  | { type: "modelChange"; key: string; modelName: string }
  | { type: "user"; key: string; turnIndex: number }
  | {
      type: "display";
      key: string;
      turnIndex: number;
      displayIndex: number;
      responseAnchor: boolean;
    }
  | {
      type: "activityAtom";
      key: string;
      turnIndex: number;
      displayIndex: number;
      atomIndex: number;
      responseAnchor: boolean;
    }
  | {
      type: "turnMeta";
      key: string;
      turnIndex: number;
      responseAnchor: boolean;
    }
  | { type: "running"; key: "running"; startedAt: number }
  | { type: "pad"; key: "pad" };

/** Flattens turns into independently measurable rows so a long live tool list can window. */
export function buildMessageListRows(
  turns: ChatTurn[],
  modelChanges: ChatModelChange[],
  showRunning: boolean,
): MessageListRow[] {
  const rows: MessageListRow[] = [];
  for (let turnIndex = 0; turnIndex < turns.length; turnIndex += 1) {
    const turn = turns[turnIndex]!;
    for (const change of modelChangesAt(modelChanges, turnIndex)) {
      rows.push({
        type: "modelChange",
        key: `model:${change.id}`,
        modelName: change.modelName,
      });
    }
    rows.push({ type: "user", key: `${turn.id}:user`, turnIndex });
    const showResponse = turn.items.length > 0 || turn.status !== "streaming";
    if (!showResponse) {
      continue;
    }
    const displayItems = buildTurnDisplayItems(turn.items, turn.status);
    let responseAnchor = true;
    const markAnchor = (): boolean => {
      if (!responseAnchor) {
        return false;
      }
      responseAnchor = false;
      return true;
    };
    displayItems.forEach((item, displayIndex) => {
      if (item.kind === "activityPhase" && item.live) {
        item.items.forEach((atom, atomIndex) => {
          rows.push({
            type: "activityAtom",
            key: `${turn.id}:atom:${atom.id}`,
            turnIndex,
            displayIndex,
            atomIndex,
            responseAnchor: markAnchor(),
          });
        });
        return;
      }
      rows.push({
        type: "display",
        key: `${turn.id}:display:${item.id}`,
        turnIndex,
        displayIndex,
        responseAnchor: markAnchor(),
      });
    });
    if (turn.status !== "streaming") {
      rows.push({
        type: "turnMeta",
        key: `${turn.id}:meta`,
        turnIndex,
        responseAnchor: markAnchor(),
      });
    }
  }
  for (const change of modelChangesAt(modelChanges, turns.length)) {
    rows.push({
      type: "modelChange",
      key: `model:${change.id}`,
      modelName: change.modelName,
    });
  }
  const lastTurn = turns.at(-1);
  if (showRunning && lastTurn !== undefined) {
    rows.push({
      type: "running",
      key: "running",
      startedAt: lastTurn.createdAt,
    });
  }
  rows.push({ type: "pad", key: "pad" });
  return rows;
}

/** Selects the switches recorded after a given number of turns. */
export function modelChangesAt(
  modelChanges: ChatModelChange[],
  turnCount: number,
): ChatModelChange[] {
  return modelChanges.filter((change) => change.afterTurnCount === turnCount);
}

/** Maps a navigator prompt/response id onto the flattened row that should receive the jump. */
export function rowIndexForAnchor(
  rows: MessageListRow[],
  turns: ChatTurn[],
  anchorId: string,
): number {
  const isUser = anchorId.endsWith(":user");
  const turnId = isUser
    ? anchorId.slice(0, -":user".length)
    : anchorId.slice(0, -":response".length);
  const turnIndex = turns.findIndex((turn) => turn.id === turnId);
  if (turnIndex < 0) {
    return -1;
  }
  if (isUser) {
    return rows.findIndex(
      (row) => row.type === "user" && row.turnIndex === turnIndex,
    );
  }
  return rows.findIndex(
    (row) =>
      (row.type === "display" ||
        row.type === "activityAtom" ||
        row.type === "turnMeta") &&
      row.turnIndex === turnIndex,
  );
}

/** Placeholder heights until `measureElement` replaces them with the real card size. */
export function estimateMessageListRowSize(row: MessageListRow): number {
  switch (row.type) {
    case "modelChange":
      return 56;
    case "user":
      return 88;
    case "display":
      return 140;
    case "activityAtom":
      return 48;
    case "turnMeta":
      return 28;
    case "running":
      return 56;
    case "pad":
      return 32;
  }
}
