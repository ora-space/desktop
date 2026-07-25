import { useEffect } from "react";
import type { ChatToolCall, ChatTurn } from "@ora/chat";
import { getRun, useWorkflowStore } from "../../state/stores/workflow-store";
import { parseOpenSpecStatus } from "./openspec-status";

/**
 * Reflects OpenSpec status JSON found in the agent's tool calls into the given
 * session's workflow run. Enhancement only: the stepper's truth is user-driven,
 * so a stream that never surfaces parseable status simply leaves it untouched.
 */
export function useWorkflowDetection(key: string, turns: readonly ChatTurn[]): void {
  const active = useWorkflowStore((state) => getRun(state, key).active);
  const setDetected = useWorkflowStore((state) => state.setDetected);

  useEffect(() => {
    if (!active) return;
    const toolCalls: ChatToolCall[] = turns.flatMap((turn) =>
      turn.items.filter((item): item is ChatToolCall => item.kind === "toolCall"),
    );
    const status = parseOpenSpecStatus(toolCalls);
    if (status !== null) setDetected(key, status);
  }, [active, key, turns, setDetected]);
}
