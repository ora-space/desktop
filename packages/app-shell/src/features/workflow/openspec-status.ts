import type { ChatToolCall } from "@ora/chat";
import type { OpenSpecStatus } from "../../state/stores/workflow-store";

/**
 * Best-effort extraction of `openspec status --json` output from the agent's tool
 * calls. It is deliberately defensive: OpenCode may deliver command output as a
 * text content block, as `rawOutput`, or as an opaque terminal reference that
 * carries no inline text (see M4 notes) — so a null result is normal, not an error.
 */

function isStatusShape(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== "object") return false;
  const artifacts = (value as Record<string, unknown>).artifacts;
  if (!Array.isArray(artifacts)) return false;
  return artifacts.every((entry) => {
    if (entry === null || typeof entry !== "object") return false;
    const record = entry as Record<string, unknown>;
    return typeof record.id === "string" && typeof record.status === "string";
  });
}

function normalize(value: Record<string, unknown>): OpenSpecStatus {
  const artifacts = (value.artifacts as Array<Record<string, unknown>>).map((entry) => ({
    id: String(entry.id),
    status: String(entry.status),
  }));
  return {
    changeName: typeof value.changeName === "string" ? value.changeName : undefined,
    artifacts,
    isComplete: value.isComplete === true,
  };
}

/** Finds a status JSON object inside free-form text (stdout may be pure JSON or wrapped). */
function extractFromText(text: string): OpenSpecStatus | null {
  if (!text.includes("artifacts")) return null;
  const candidates = [text.trim()];
  const start = text.indexOf("{");
  const end = text.lastIndexOf("}");
  if (start !== -1 && end > start) candidates.push(text.slice(start, end + 1));
  for (const candidate of candidates) {
    try {
      const parsed: unknown = JSON.parse(candidate);
      if (isStatusShape(parsed)) return normalize(parsed);
    } catch {
      // Candidate was not valid JSON; try the next slice.
    }
  }
  return null;
}

function fromToolCall(toolCall: ChatToolCall): OpenSpecStatus | null {
  const { rawOutput, content } = toolCall;
  if (isStatusShape(rawOutput)) return normalize(rawOutput as Record<string, unknown>);
  const texts: string[] = [];
  if (typeof rawOutput === "string") texts.push(rawOutput);
  for (const item of content) {
    if (item.type === "content") {
      const block = item.content as { text?: unknown } | undefined;
      if (block !== undefined && typeof block.text === "string") texts.push(block.text);
    }
  }
  for (const text of texts) {
    const status = extractFromText(text);
    if (status !== null) return status;
  }
  return null;
}

/** The most recent parseable OpenSpec status across the given tool calls, or null. */
export function parseOpenSpecStatus(toolCalls: readonly ChatToolCall[]): OpenSpecStatus | null {
  for (let index = toolCalls.length - 1; index >= 0; index -= 1) {
    const status = fromToolCall(toolCalls[index]);
    if (status !== null) return status;
  }
  return null;
}
