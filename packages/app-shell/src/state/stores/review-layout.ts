/** Fallback opening width when the host cannot be measured (e.g. tests). */
export const DEFAULT_REVIEW_WIDTH = 640;
/** Narrowest review width a user resize settles on; below it the panel collapses. */
export const MIN_REVIEW_WIDTH = 460;
/** Caps the panel so an ultrawide window cannot push the conversation aside. */
export const MAX_REVIEW_WIDTH = 1200;
/** Matches the conversation panel's min so the chat never loses its floor. */
export const MIN_CONVERSATION_WIDTH = 360;
/** Host width below which the panel opens at a leaner ratio (small windows). */
const SMALL_HOST_WIDTH = 1200;

/**
 * Picks the opening width from the space next to the conversation: a leaner
 * ratio on ordinary windows, a fuller one once the host is maximized-wide, all
 * clamped so the chat keeps at least its minimum.
 */
export function responsiveReviewWidth(available: number): number {
  if (available <= 0) return DEFAULT_REVIEW_WIDTH;
  const ratio = available <= SMALL_HOST_WIDTH ? 0.4 : 0.45;
  const ideal = Math.round(available * ratio);
  const conversationFloor = Math.max(0, available - MIN_CONVERSATION_WIDTH);
  return Math.min(
    Math.max(MIN_REVIEW_WIDTH, ideal),
    MAX_REVIEW_WIDTH,
    conversationFloor,
  );
}
