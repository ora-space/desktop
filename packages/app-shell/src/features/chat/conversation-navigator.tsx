import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { IconChevronDown, IconChevronUp } from "@tabler/icons-react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import type { ChatTurn } from "@ora/chat";

interface ConversationNavigatorProps {
  turns: ChatTurn[];
  activeAnchorId: string | null;
  onNavigate: (anchorId: string) => void;
}

interface ConversationAnchor {
  id: string;
  label: string;
  preview: string;
  summary: string;
  role: "user" | "assistant";
}

interface AnchorPreview {
  anchorId: string;
  anchorLeft: number;
  anchorTop: number;
  left?: number;
  top?: number;
}

const PREVIEW_MAX_CHARACTERS = 120;
const PREVIEW_GAP_PX = 8;
const VIEWPORT_MARGIN_PX = 12;
const WHEEL_ANCHORS_PER_STEP = 1;
const TRACK_SHIFT_DURATION_MS = 240;
const NEW_ANCHOR_DURATION_MS = 180;
const ANCHOR_WIDTH = {
  user: "28%",
  assistant: "46%",
  emphasized: "58%",
} as const;

/** Renders a Grok-style minimap with separate beats for prompts and responses. */
export function ConversationNavigator({ turns, activeAnchorId, onNavigate }: ConversationNavigatorProps) {
  const { t } = useTranslation();
  const anchorListRef = useRef<HTMLDivElement>(null);
  const anchorTrackRef = useRef<HTMLDivElement>(null);
  const previewRef = useRef<HTMLDivElement>(null);
  const pointerPositionRef = useRef<{ y: number } | null>(null);
  const previewSyncFrameRef = useRef<number | null>(null);
  const previousAnchorCountRef = useRef<number | null>(null);
  const previousTrackHeightRef = useRef(0);
  const [preview, setPreview] = useState<AnchorPreview | null>(null);
  const anchors = conversationAnchors(turns, t);
  const previewAnchorId = preview?.anchorId ?? null;
  const previewAnchor = anchors.find((anchor) => anchor.id === previewAnchorId);
  const lastGeneratedAnchorId = anchors.at(-1)?.id;

  /** Positions the preview beside the actual tick while keeping it inside the viewport. */
  const showPreview = useCallback((anchorId: string, target: HTMLElement) => {
    const bounds = target.getBoundingClientRect();
    const anchorTop = bounds.top + bounds.height / 2;
    setPreview((current) => {
      if (current?.anchorId === anchorId && current.anchorLeft === bounds.left && current.anchorTop === anchorTop) {
        return current;
      }
      return {
        anchorId,
        anchorLeft: bounds.left,
        anchorTop,
        left: current?.left,
        top: current?.top,
      };
    });
  }, []);

  /** Re-resolves the tick beneath a stationary pointer after the scroll track moves. */
  const syncPreviewWithPointer = useCallback(() => {
    if (!anchorListRef.current || !pointerPositionRef.current) {
      setPreview(null);
      return;
    }
    if (previewSyncFrameRef.current !== null) window.cancelAnimationFrame(previewSyncFrameRef.current);
    previewSyncFrameRef.current = window.requestAnimationFrame(() => {
      previewSyncFrameRef.current = null;
      const list = anchorListRef.current;
      const pointer = pointerPositionRef.current;
      if (!list || !pointer) {
        setPreview(null);
        return;
      }

      const hoveredTick = Array.from(list.querySelectorAll<HTMLElement>("[data-conversation-tick]")).find((tick) => {
        const bounds = tick.getBoundingClientRect();
        return pointer.y >= bounds.top && pointer.y < bounds.bottom;
      });
      const anchorId = hoveredTick?.dataset.conversationAnchorId;
      if (hoveredTick && anchorId) showPreview(anchorId, hoveredTick);
      else setPreview(null);
    });
  }, [showPreview]);

  useLayoutEffect(() => {
    const list = anchorListRef.current;
    const track = anchorTrackRef.current;
    const activeButton = anchorListRef.current?.querySelector<HTMLElement>('[aria-current="location"]');
    if (!list || !track) return;

    const previousCount = previousAnchorCountRef.current;
    const addedCount = previousCount === null ? 0 : Math.max(0, anchors.length - previousCount);
    const previousHeight = previousTrackHeightRef.current;
    const trackHeight = track.scrollHeight;
    const reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

    if (addedCount > 0 && activeAnchorId === lastGeneratedAnchorId) {
      const overflows = list.scrollHeight > list.clientHeight;
      if (overflows) {
        list.scrollTop = list.scrollHeight - list.clientHeight;
        const shift = Math.max(0, trackHeight - previousHeight);
        if (!reduceMotion && shift > 0 && typeof track.animate === "function") {
          track.animate(
            [{ transform: `translateY(${shift}px)` }, { transform: "translateY(0)" }],
            { duration: TRACK_SHIFT_DURATION_MS, easing: "cubic-bezier(0.22, 1, 0.36, 1)" },
          );
        }
      } else if (activeButton && typeof activeButton.scrollIntoView === "function") {
        activeButton.scrollIntoView({ block: "nearest", behavior: reduceMotion ? "auto" : "smooth" });
      }

      if (!reduceMotion) {
        const newButtons = Array.from(track.querySelectorAll<HTMLElement>("[data-conversation-tick]")).slice(-addedCount);
        for (const button of newButtons) {
          if (typeof button.animate !== "function") continue;
          button.animate(
            [{ opacity: 0, transform: "translateY(3px)" }, { opacity: 1, transform: "translateY(0)" }],
            { duration: NEW_ANCHOR_DURATION_MS, easing: "ease-out" },
          );
        }
      }
    } else if (activeButton && typeof activeButton.scrollIntoView === "function") {
      activeButton.scrollIntoView({ block: "nearest", behavior: reduceMotion ? "auto" : "smooth" });
    }

    previousAnchorCountRef.current = anchors.length;
    previousTrackHeightRef.current = trackHeight;
  }, [activeAnchorId, anchors.length, lastGeneratedAnchorId]);

  useEffect(() => {
    const list = anchorListRef.current;
    if (!list) return;
    const handleWheel = (event: WheelEvent) => {
      if (list.scrollHeight <= list.clientHeight || event.deltaY === 0) return;
      event.preventDefault();
      event.stopPropagation();
      const anchorHeight = list.querySelector<HTMLElement>("[data-conversation-tick]")?.offsetHeight ?? 0;
      const maxStep = anchorHeight > 0 ? anchorHeight * WHEEL_ANCHORS_PER_STEP : list.clientHeight / 4;
      const previousScrollTop = list.scrollTop;
      list.scrollTop += Math.max(-maxStep, Math.min(maxStep, event.deltaY));
      if (list.scrollTop !== previousScrollTop) syncPreviewWithPointer();
    };
    list.addEventListener("wheel", handleWheel, { passive: false });
    return () => {
      list.removeEventListener("wheel", handleWheel);
      if (previewSyncFrameRef.current !== null) window.cancelAnimationFrame(previewSyncFrameRef.current);
    };
  }, [anchors.length, syncPreviewWithPointer]);

  useLayoutEffect(() => {
    const element = previewRef.current;
    if (!preview || !element) return;
    const bounds = element.getBoundingClientRect();
    const halfHeight = bounds.height / 2;
    const left = Math.max(VIEWPORT_MARGIN_PX, preview.anchorLeft - PREVIEW_GAP_PX - bounds.width);
    const top = Math.min(
        window.innerHeight - VIEWPORT_MARGIN_PX - halfHeight,
        Math.max(VIEWPORT_MARGIN_PX + halfHeight, preview.anchorTop),
      );
    setPreview((current) => {
      if (current !== preview || current.left === left && current.top === top) return current;
      return { ...current, left, top };
    });
  }, [preview]);

  if (turns.length < 3) return null;

  const matchedActiveIndex = anchors.findIndex((anchor) => anchor.id === activeAnchorId);
  const activeIndex = matchedActiveIndex >= 0 ? matchedActiveIndex : Math.max(0, anchors.length - 1);

  /** Moves through prompts and responses in the same order as the visible anchor track. */
  const navigateByAnchor = (offset: -1 | 1) => {
    const nextAnchor = anchors[activeIndex + offset];
    if (nextAnchor) onNavigate(nextAnchor.id);
  };

  return (
    <>
      <nav
        aria-label={t("chat.historyNavigation")}
        className="group/history-nav pointer-events-none fixed right-1.5 top-1/2 z-20 hidden -translate-y-1/2 sm:block"
      >
        <div className="pointer-events-auto relative flex w-7 flex-col items-center">
        <button
          type="button"
          aria-label={t("chat.previousMessage")}
          disabled={activeIndex === 0}
          onClick={() => navigateByAnchor(-1)}
          className="mb-px flex size-6 cursor-pointer items-center justify-center rounded-md text-muted-foreground opacity-0 outline-none transition-[color,background-color,opacity] duration-150 group-hover/history-nav:opacity-100 group-focus-within/history-nav:opacity-100 hover:bg-muted/70 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:invisible"
        >
          <IconChevronUp className="size-3.5" />
        </button>

        <div
          ref={anchorListRef}
          data-testid="conversation-anchor-list"
          onMouseMove={(event) => {
            pointerPositionRef.current = { y: event.clientY };
          }}
          onMouseLeave={() => {
            pointerPositionRef.current = null;
            setPreview(null);
          }}
          onScroll={syncPreviewWithPointer}
          className="scrollbar-hide max-h-48 overflow-y-auto overscroll-contain py-px"
        >
          <div ref={anchorTrackRef} data-testid="conversation-anchor-track" className="flex flex-col items-end">
          {anchors.map((anchor) => {
            const active = anchor.id === activeAnchorId;
            const previewed = anchor.id === previewAnchorId;
            const baseWidth = ANCHOR_WIDTH[anchor.role];

            return (
              <button
                key={anchor.id}
                type="button"
                aria-label={t("chat.jumpToAnchor", { label: anchor.label, message: anchor.summary })}
                aria-current={active ? "location" : undefined}
                onMouseEnter={(event) => {
                  pointerPositionRef.current = { y: event.clientY };
                  showPreview(anchor.id, event.currentTarget);
                }}
                onFocus={(event) => showPreview(anchor.id, event.currentTarget)}
                onBlur={() => setPreview(null)}
                onClick={() => onNavigate(anchor.id)}
                data-conversation-tick
                data-conversation-anchor-id={anchor.id}
                className="group/tick relative flex h-4 w-7 shrink-0 cursor-pointer items-center justify-end rounded-md pr-1 outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span
                  aria-hidden="true"
                  className={`h-px origin-right rounded-full transition-[width,background-color,opacity] duration-200 ease-out motion-reduce:transition-none ${active ? "bg-foreground/85" : anchor.role === "user" ? "bg-muted-foreground/65" : "bg-muted-foreground/45 group-hover/tick:bg-foreground/70"}`}
                  style={{
                    width: active || previewed ? ANCHOR_WIDTH.emphasized : baseWidth,
                    opacity: previewAnchorId === null || previewed ? 1 : 0.72,
                  }}
                />
              </button>
            );
          })}
          </div>
        </div>

        <button
          type="button"
          aria-label={t("chat.nextMessage")}
          disabled={activeIndex === anchors.length - 1}
          onClick={() => navigateByAnchor(1)}
          className="mt-px flex size-6 cursor-pointer items-center justify-center rounded-md text-muted-foreground opacity-0 outline-none transition-[color,background-color,opacity] duration-150 group-hover/history-nav:opacity-100 group-focus-within/history-nav:opacity-100 hover:bg-muted/70 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:invisible"
        >
          <IconChevronDown className="size-3.5" />
        </button>
        </div>
      </nav>
      {preview && previewAnchor && createPortal(
        <div
          ref={previewRef}
          data-testid="conversation-anchor-preview"
          className={`pointer-events-none fixed z-30 w-56 max-w-[calc(100vw-var(--spacing)*6)] -translate-y-1/2 rounded-lg border border-border/70 bg-popover/95 p-3 text-left text-popover-foreground shadow-lg backdrop-blur-md ${preview.left === undefined ? "invisible" : "animate-preview-fade-in motion-reduce:animate-none"}`}
          style={{ left: preview.left ?? preview.anchorLeft, top: preview.top ?? preview.anchorTop }}
        >
          <p className="mb-1 text-[11px] text-muted-foreground">{previewAnchor.label}</p>
          <p className="line-clamp-3 max-h-15 overflow-hidden text-xs leading-5 break-words [overflow-wrap:anywhere]">{previewAnchor.preview}</p>
        </div>,
        document.body,
      )}
    </>
  );
}

/** Builds stable prompt/response pairs so line length communicates message role. */
function conversationAnchors(turns: ChatTurn[], t: ReturnType<typeof useTranslation>["t"]): ConversationAnchor[] {
  return turns.flatMap((turn, index) => {
    const number = index + 1;
    const userSummary = summarizeMessage(turn.userMessage.content, t("chat.untitledTurn", { index: number }));
    const userAnchor: ConversationAnchor = {
      id: `${turn.id}:user`,
      label: t("chat.userAnchorLabel", { index: number }),
      preview: truncatePreview(userSummary),
      summary: userSummary,
      role: "user",
    };
    if (turn.items.length === 0 && turn.status === "streaming") return [userAnchor];
    const assistantSummary = responseSummary(turn, t("chat.assistantReplied"));
    return [
      userAnchor,
      {
        id: `${turn.id}:response`,
        label: t("chat.responseAnchorLabel", { index: number }),
        preview: truncatePreview(assistantSummary),
        summary: assistantSummary,
        role: "assistant" as const,
      },
    ];
  });
}

/** Chooses the latest readable Agent activity for the response preview. */
function responseSummary(turn: ChatTurn, fallback: string): string {
  for (const item of [...turn.items].reverse()) {
    switch (item.kind) {
      case "message":
      case "thought":
        return summarizeMessage(item.content, fallback);
      case "toolCall":
        return item.title;
      case "plan":
        return item.entries.at(-1)?.content ?? fallback;
      case "unsupportedContent":
        continue;
    }
  }
  return fallback;
}

/** Reduces multiline content to one useful navigation label. */
function summarizeMessage(content: string, fallback: string): string {
  const normalized = content.replace(/\s+/g, " ").trim();
  return normalized || fallback;
}

/** Caps preview payloads while preserving the full summary for accessibility. */
function truncatePreview(summary: string): string {
  const characters = Array.from(summary);
  if (characters.length <= PREVIEW_MAX_CHARACTERS) return summary;
  return `${characters.slice(0, PREVIEW_MAX_CHARACTERS - 3).join("")}...`;
}
