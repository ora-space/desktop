import {
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  type RefObject,
} from "react";

const NAVIGATION_TOP_OFFSET_PX = 12;
const TAIL_PROXIMITY_PX = 24;
const NAVIGATION_ARRIVAL_TOLERANCE_PX = 1;

/** Stable role-aware item used by both the full chat and embedded node views. */
export interface ConversationAnchor {
  id: string;
  label: string;
  preview: string;
  summary: string;
  role: "user" | "assistant";
}

export interface ConversationNavigationOptions {
  /** Scroll viewport containing elements marked with `data-conversation-anchor`. */
  scrollRef: RefObject<HTMLDivElement | null>;
  /** Optional content wrapper used to keep the live tail stable after Markdown layout. */
  contentRef?: RefObject<HTMLDivElement | null>;
  /** Stable key for a new user turn; assistant streaming updates should not reset reading position. */
  followTailKey: string;
  /** Last anchor currently represented by the projection. */
  lastAnchorId: string | null;
}

export interface ConversationNavigationResult {
  activeAnchorId: string | null;
  isAtTail: boolean;
  handleScroll: () => void;
  handleWheel: (deltaY: number) => void;
  beginPointerScroll: () => void;
  endPointerScroll: () => void;
  navigateToAnchor: (anchorId: string) => void;
  navigateToTail: () => void;
}

/**
 * Shares tail-following, manual-scroll detection, anchor jumps, and focus
 * highlighting between the full conversation and embedded node sessions.
 */
export function useConversationNavigation({
  scrollRef,
  contentRef,
  followTailKey,
  lastAnchorId,
}: ConversationNavigationOptions): ConversationNavigationResult {
  const followTailRef = useRef(true);
  const pointerScrollRef = useRef(false);
  const pendingNavigationRef = useRef<{ scrollTop: number } | null>(null);
  const [isAtTail, setIsAtTail] = useState(true);
  const [navigation, setNavigation] = useState<{
    activeAnchorId: string | null;
    lastAnchorId: string | null;
  }>({
    activeAnchorId: lastAnchorId,
    lastAnchorId,
  });
  const activeAnchorId = navigation.lastAnchorId === lastAnchorId
    ? navigation.activeAnchorId
    : lastAnchorId;

  const cancelPendingNavigation = useCallback(() => {
    pendingNavigationRef.current = null;
  }, []);

  const handleScroll = useCallback(() => {
    const element = scrollRef.current;
    if (!element) return;

    const nextIsAtTail = element.scrollHeight - element.scrollTop - element.clientHeight < TAIL_PROXIMITY_PX;
    setIsAtTail(nextIsAtTail);
    const pendingNavigation = pendingNavigationRef.current;
    if (pendingNavigation) {
      const maximumScrollTop = Math.max(0, element.scrollHeight - element.clientHeight);
      const destination = Math.min(pendingNavigation.scrollTop, maximumScrollTop);
      if (Math.abs(element.scrollTop - destination) <= NAVIGATION_ARRIVAL_TOLERANCE_PX) {
        pendingNavigationRef.current = null;
      }
      return;
    }

    if (nextIsAtTail) followTailRef.current = true;
    else if (pointerScrollRef.current) followTailRef.current = false;
    const nextAnchorId = findActiveAnchorId(element);
    setNavigation((current) => (
      current.activeAnchorId === nextAnchorId && current.lastAnchorId === lastAnchorId
        ? current
        : { activeAnchorId: nextAnchorId, lastAnchorId }
    ));
  }, [lastAnchorId, scrollRef]);

  const handleWheel = useCallback((deltaY: number) => {
    cancelPendingNavigation();
    if (deltaY < 0) followTailRef.current = false;
  }, [cancelPendingNavigation]);

  const beginPointerScroll = useCallback(() => {
    cancelPendingNavigation();
    pointerScrollRef.current = true;
  }, [cancelPendingNavigation]);

  const endPointerScroll = useCallback(() => {
    pointerScrollRef.current = false;
  }, []);

  useLayoutEffect(() => {
    followTailRef.current = true;
    const element = scrollRef.current;
    if (!element) return;
    element.style.scrollBehavior = "auto";
    element.scrollTop = element.scrollHeight;
  }, [followTailKey, scrollRef]);

  useLayoutEffect(() => {
    const content = contentRef?.current;
    if (!content || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      const element = scrollRef.current;
      if (!element || !followTailRef.current) return;
      element.scrollTop = element.scrollHeight;
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [contentRef, scrollRef]);

  const navigateToAnchor = useCallback((anchorId: string) => {
    const element = scrollRef.current;
    if (!element) return;
    const anchor = Array.from(element.querySelectorAll<HTMLElement>("[data-conversation-anchor]"))
      .find((candidate) => candidate.dataset.conversationAnchor === anchorId);
    if (!anchor) return;

    followTailRef.current = false;
    const top = Math.max(0, anchor.offsetTop - NAVIGATION_TOP_OFFSET_PX);
    pendingNavigationRef.current = { scrollTop: top };
    setNavigation({ activeAnchorId: anchorId, lastAnchorId });
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const behavior = reduceMotion ? "auto" : "smooth";
    if (typeof element.scrollTo === "function") element.scrollTo({ top, behavior });
    else element.scrollTop = top;
    highlightConversationAnchor(anchor, reduceMotion);
  }, [lastAnchorId, scrollRef]);

  const navigateToTail = useCallback(() => {
    const element = scrollRef.current;
    if (!element) return;
    followTailRef.current = true;
    pendingNavigationRef.current = { scrollTop: element.scrollHeight };
    setNavigation({ activeAnchorId: lastAnchorId, lastAnchorId });
    const behavior = window.matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth";
    if (typeof element.scrollTo === "function") element.scrollTo({ top: element.scrollHeight, behavior });
    else element.scrollTop = element.scrollHeight;
  }, [lastAnchorId, scrollRef]);

  return {
    activeAnchorId,
    isAtTail,
    handleScroll,
    handleWheel,
    beginPointerScroll,
    endPointerScroll,
    navigateToAnchor,
    navigateToTail,
  };
}

/** Finds the prompt or response aligned with the viewport-top reading line. */
function findActiveAnchorId(element: HTMLDivElement): string | null {
  const anchors = Array.from(element.querySelectorAll<HTMLElement>("[data-conversation-anchor]"));
  if (anchors.length === 0) return null;
  if (element.scrollHeight - element.scrollTop - element.clientHeight < TAIL_PROXIMITY_PX) {
    return anchors.at(-1)?.dataset.conversationAnchor ?? null;
  }

  const readingLine = element.scrollTop + NAVIGATION_TOP_OFFSET_PX;
  let activeAnchorId = anchors[0]?.dataset.conversationAnchor ?? null;
  for (const anchor of anchors) {
    if (anchor.offsetTop > readingLine) break;
    activeAnchorId = anchor.dataset.conversationAnchor ?? activeAnchorId;
  }
  return activeAnchorId;
}

/** Briefly outlines an anchor after a navigator jump. */
function highlightConversationAnchor(anchor: HTMLElement, reduceMotion: boolean): void {
  const outline = anchor.querySelector<HTMLElement>("[data-anchor-highlight]");
  if (!outline || typeof outline.animate !== "function") return;
  if (typeof outline.getAnimations === "function") {
    outline.getAnimations().forEach((animation) => animation.cancel());
  }
  // Opacity-only ring: avoids SVG stroke-dash partial draws that showed up as
  // "half outlines" on both the full chat and embedded node sessions.
  outline.animate(
    reduceMotion
      ? [
          { opacity: 0.82 },
          { opacity: 0 },
        ]
      : [
          { opacity: 0, offset: 0 },
          { opacity: 0.95, offset: 0.08 },
          { opacity: 0.95, offset: 0.7 },
          { opacity: 0, offset: 1 },
        ],
    { duration: reduceMotion ? 250 : 2500, easing: "cubic-bezier(0.22, 1, 0.36, 1)" },
  );
}
