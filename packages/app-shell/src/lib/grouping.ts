import type { Conversation } from "./types";

/** A labeled bucket of conversations rendered as a sidebar section. */
export interface ConversationGroup {
  label: string;
  conversations: Conversation[];
}

const DAY = 24 * 60 * 60 * 1000;

/** Returns the epoch ms of midnight local time for the given timestamp. */
function startOfDay(timestamp: number): number {
  const date = new Date(timestamp);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

/**
 * Buckets conversations by recency of `updatedAt` into ChatGPT-style sidebar
 * sections (Today, Yesterday, Previous 7 Days, …). Each bucket is sorted
 * newest-first; empty buckets are omitted.
 */
export function groupConversationsByDate(
  conversations: Conversation[],
  now: number,
): ConversationGroup[] {
  const todayStart = startOfDay(now);
  const buckets = [
    { label: "Today", min: todayStart, max: todayStart + DAY, items: [] as Conversation[] },
    { label: "Yesterday", min: todayStart - DAY, max: todayStart, items: [] as Conversation[] },
    { label: "Previous 7 Days", min: todayStart - 7 * DAY, max: todayStart - DAY, items: [] as Conversation[] },
    { label: "Previous 30 Days", min: todayStart - 30 * DAY, max: todayStart - 7 * DAY, items: [] as Conversation[] },
    { label: "Older", min: Number.NEGATIVE_INFINITY, max: todayStart - 30 * DAY, items: [] as Conversation[] },
  ];

  for (const conversation of conversations) {
    const bucket = buckets.find((b) => conversation.updatedAt >= b.min && conversation.updatedAt < b.max);
    (bucket ?? buckets[buckets.length - 1]!).items.push(conversation);
  }

  for (const bucket of buckets) {
    bucket.items.sort((a, z) => z.updatedAt - a.updatedAt);
  }

  return buckets
    .filter((bucket) => bucket.items.length > 0)
    .map(({ label, items }) => ({ label, conversations: items }));
}
