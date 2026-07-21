import type { ChatToolCall } from "@ora/chat";

export type ToolCallGroupKind = "exploration" | "changes" | "commands";

/** Maps tool kinds into the activity groups that are safe to summarize together. */
export function toolCallGroupKind(tool: ChatToolCall): ToolCallGroupKind | null {
  switch (tool.toolKind) {
    case "read":
    case "search":
    case "fetch":
      return "exploration";
    case "edit":
    case "move":
    case "delete":
      return "changes";
    case "execute":
      return "commands";
    case "think":
    case "switch_mode":
    case "other":
    case undefined:
      return null;
  }
}
