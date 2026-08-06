/** Builds a compact direct-chat title from the first message without splitting Unicode characters. */
export function directChatTitle(text: string): string {
  const normalized = text.trim().replace(/\s+/gu, " ");
  return Array.from(normalized).slice(0, 10).join("");
}
