import type { ChatMessage, Conversation, CurrentUser, Role } from "./types";

/** The signed-in user rendered in the sidebar footer. */
export const CURRENT_USER: CurrentUser = {
  name: "Eric Wang",
  email: "eric@ora.dev",
};

const MINUTE = 60 * 1000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** Generates a fresh id for client-created entities. */
export function createId(): string {
  return crypto.randomUUID();
}

/** Builds a message offset from a conversation's base time. */
function message(role: Role, content: string, createdAt: number): ChatMessage {
  return { id: createId(), role, content, createdAt };
}

/** Builds a seed conversation with messages spaced a few minutes apart. */
function seedConversation(
  id: string,
  title: string,
  base: number,
  exchanges: Array<[user: string, assistant: string]>,
): Conversation {
  const messages: ChatMessage[] = [];
  let cursor = base;
  for (const [userText, assistantText] of exchanges) {
    messages.push(message("user", userText, cursor));
    cursor += 2 * MINUTE;
    messages.push(message("assistant", assistantText, cursor));
    cursor += 4 * MINUTE;
  }
  return {
    id,
    title,
    messages,
    createdAt: base,
    updatedAt: messages[messages.length - 1]!.createdAt,
  };
}

/**
 * Seed conversations used on first load (before the user creates any of their
 * own). Timestamps are computed relative to load time so the sidebar's date
 * buckets stay meaningful as days pass.
 */
export function createSeedConversations(now: number): Conversation[] {
  return [
    seedConversation("seed-runtime", "Refactor the agent runtime", now - 2 * HOUR, [
      [
        "Can we split the agent loop out of the session handler?",
        "Yes — extracting an `AgentLoop` trait behind the session handler keeps the transport concerns separate from the step orchestration. I'd start by moving `advance_step` and its retry policy into the new module.",
      ],
      [
        "What about the streaming channel?",
        "Keep it on the session side for now. The loop should accept an opaque `EventSink` so we can swap transports without touching the step logic.",
      ],
    ]),
    seedConversation("seed-web-client", "Design the web client layout", now - 5 * HOUR, [
      [
        "Draft a layout for the web client.",
        "A collapsible conversation sidebar on the left and a centered composer on the right mirrors the ChatGPT flow and keeps the focus on the conversation.",
      ],
    ]),
    seedConversation("seed-worktree", "Investigate the worktree leak", now - 26 * HOUR, [
      [
        "We're leaking worktrees when a task is cancelled mid-flight.",
        "The cancellation path drops the worktree handle before the agent finishes flushing. Moving cleanup into a `Drop` guard on the task context should close that gap.",
      ],
    ]),
    seedConversation("seed-contracts", "Draft contracts for sessions", now - 4 * DAY, [
      [
        "What does the session contract need to expose?",
        "Status, the owning task, and the agent session id. The terminal attach route stays a separate WebSocket concern.",
      ],
    ]),
    seedConversation("seed-tauri", "Set up Tauri capabilities", now - 20 * DAY, [
      [
        "Which capabilities does the desktop shell need?",
        "Filesystem scope for the project root and a window-event channel. Everything else stays behind the HTTP API so the web client can reuse it verbatim.",
      ],
    ]),
  ];
}

const ASSISTANT_REPLIES: ReadonlyArray<(prompt: string) => string> = [
  (prompt) => `Got it — here's how I'd approach "${truncate(prompt, 80)}": break it into the smallest reversible step, ship that, then iterate. Want me to draft the first change?`,
  () => "Makes sense. I'll start by reproducing the current behavior with a small test, then make the change behind that test so we can verify it end-to-end.",
  () => "Good question. The short answer is to keep the transport out of the orchestration layer — accept an event sink and let the caller decide how to render it. Want me to sketch the interface?",
  () => "I can take that on. I'll open a worktree, make the change, and run the full suite before reporting back. Anything specific you want me to watch for?",
  () => "Here's a first pass. It keeps the public API the same and moves the tricky bit into a small pure function so it's easy to unit-test in isolation.",
];

/** Returns a deterministic-ish canned reply for a user prompt (prototype only). */
export function createAssistantReply(prompt: string): string {
  const reply = ASSISTANT_REPLIES[Math.floor(prompt.length % ASSISTANT_REPLIES.length)]!;
  return reply(prompt);
}

/** Derives a human-readable title from the first user message of a conversation. */
export function deriveTitle(firstMessage: string): string {
  const trimmed = firstMessage.trim().replace(/\s+/g, " ");
  if (trimmed.length <= 42) return trimmed || "New chat";
  return `${trimmed.slice(0, 42).trimEnd()}…`;
}

function truncate(value: string, max: number): string {
  return value.length <= max ? value : `${value.slice(0, max).trimEnd()}…`;
}
