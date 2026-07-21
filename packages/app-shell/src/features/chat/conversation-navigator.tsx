import { useEffect, useRef, useState } from "react";
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
  left: number;
  top: number;
}

const PREVIEW_MAX_CHARACTERS = 120;

/** Renders a Grok-style minimap with separate beats for prompts and responses. */
export function ConversationNavigator({ turns, activeAnchorId, onNavigate }: ConversationNavigatorProps) {
  const { t } = useTranslation();
  const anchorListRef = useRef<HTMLDivElement>(null);
  const [preview, setPreview] = useState<AnchorPreview | null>(null);
  const anchors = conversationAnchors(turns, t);
  const previewAnchorId = preview?.anchorId ?? null;
  const previewAnchor = anchors.find((anchor) => anchor.id === previewAnchorId);

  useEffect(() => {
    const activeButton = anchorListRef.current?.querySelector<HTMLElement>('[aria-current="location"]');
    if (activeButton && typeof activeButton.scrollIntoView === "function") {
      activeButton.scrollIntoView({ block: "nearest" });
    }
  }, [activeAnchorId, anchors.length]);

  if (turns.length < 3) return null;

  const activeIndex = Math.max(0, anchors.findIndex((anchor) => anchor.id === activeAnchorId));

  /** Navigates by one message anchor while keeping the control inert at either end. */
  const navigateBy = (offset: -1 | 1) => {
    const nextAnchor = anchors[activeIndex + offset];
    if (nextAnchor) onNavigate(nextAnchor.id);
  };

  /** Positions the preview beside the actual tick while keeping it inside the viewport. */
  const showPreview = (anchorId: string, target: HTMLElement) => {
    const bounds = target.getBoundingClientRect();
    const desiredTop = bounds.top + bounds.height / 2;
    setPreview({
      anchorId,
      left: Math.max(232, bounds.left - 8),
      top: Math.min(window.innerHeight - 72, Math.max(72, desiredTop)),
    });
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
          aria-label={t("chat.previousTurn")}
          disabled={activeIndex === 0}
          onClick={() => navigateBy(-1)}
          className="mb-px flex size-6 cursor-pointer items-center justify-center rounded-md text-muted-foreground opacity-0 outline-none transition-[color,background-color,opacity] duration-150 group-hover/history-nav:opacity-100 group-focus-within/history-nav:opacity-100 hover:bg-muted/70 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:invisible"
        >
          <IconChevronUp className="size-3.5" />
        </button>

        <div
          ref={anchorListRef}
          data-testid="conversation-anchor-list"
          onScroll={() => setPreview(null)}
          className="scrollbar-hide flex max-h-48 flex-col items-end overflow-y-auto overscroll-contain py-px"
        >
          {anchors.map((anchor) => {
            const active = anchor.id === activeAnchorId;
            const previewed = anchor.id === previewAnchorId;
            const baseWidth = anchor.role === "user" ? 16 : 10;

            return (
              <button
                key={anchor.id}
                type="button"
                aria-label={t("chat.jumpToAnchor", { label: anchor.label, message: anchor.summary })}
                aria-current={active ? "location" : undefined}
                onMouseEnter={(event) => showPreview(anchor.id, event.currentTarget)}
                onMouseLeave={() => setPreview(null)}
                onFocus={(event) => showPreview(anchor.id, event.currentTarget)}
                onBlur={() => setPreview(null)}
                onClick={() => onNavigate(anchor.id)}
                className="group/tick relative flex h-3 w-7 cursor-pointer items-center justify-end rounded-md pr-1 outline-none focus-visible:ring-2 focus-visible:ring-ring"
              >
                <span
                  aria-hidden="true"
                  className={`h-px origin-right rounded-full transition-[width,background-color,opacity] duration-200 ease-out motion-reduce:transition-none ${active ? "bg-foreground/85" : anchor.role === "user" ? "bg-muted-foreground/65" : "bg-muted-foreground/45 group-hover/tick:bg-foreground/70"}`}
                  style={{
                    width: active || previewed ? 20 : baseWidth,
                    opacity: previewAnchorId === null || previewed ? 1 : 0.72,
                  }}
                />
              </button>
            );
          })}
        </div>

        <button
          type="button"
          aria-label={t("chat.nextTurn")}
          disabled={activeIndex === anchors.length - 1}
          onClick={() => navigateBy(1)}
          className="mt-px flex size-6 cursor-pointer items-center justify-center rounded-md text-muted-foreground opacity-0 outline-none transition-[color,background-color,opacity] duration-150 group-hover/history-nav:opacity-100 group-focus-within/history-nav:opacity-100 hover:bg-muted/70 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:invisible"
        >
          <IconChevronDown className="size-3.5" />
        </button>
        </div>
      </nav>
      {preview && previewAnchor && createPortal(
        <div
          data-testid="conversation-anchor-preview"
          className="pointer-events-none fixed z-30 w-56 -translate-x-full -translate-y-1/2 animate-in rounded-lg border border-border/70 bg-popover/95 p-3 text-left text-popover-foreground shadow-lg backdrop-blur-md duration-150 fade-in slide-in-from-right-1 motion-reduce:animate-none"
          style={{ left: preview.left, top: preview.top }}
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
