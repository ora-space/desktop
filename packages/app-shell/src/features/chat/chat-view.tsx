import { useLayoutEffect, useRef, type ReactNode } from "react";
import { Composer } from "./composer";
import { LandingHeading, LandingSuggestions } from "./empty-state";
import { MessageList } from "./message-list";
import { Tooltip, TooltipContent, TooltipTrigger } from "@ora/ui";
import type { ChatMessage } from "@ora/chat";

interface ChatViewProps {
  messages: ChatMessage[];
  userName: string;
  isResponding: boolean;
  error: string | null;
  disabled?: boolean;
  onSend: (text: string) => void;
  /**
   * Optional strip rendered directly above the composer. Passed in rather than
   * built here so the chat pane stays unaware of workspace entities.
   */
  contextBar?: ReactNode;
  /**
   * Why the composer is disabled, surfaced on hover. Preferred over an inline
   * message for a state the user can fix from the context bar right below it.
   */
  disabledHint?: string;
}

/** How long the composer takes to travel between the landing and thread layouts. */
const SLIDE_DURATION_MS = 420;
/** Decelerating curve: quick departure, soft landing, no overshoot. */
const SLIDE_EASING = "cubic-bezier(0.32, 0.72, 0, 1)";

/**
 * The right pane. The composer keeps a single DOM node across the empty and
 * thread layouts so sending the first message slides it down to the bottom
 * instead of tearing it down and rebuilding it in the new position.
 */
export function ChatView({ messages, userName, isResponding, error, disabled = false, onSend, contextBar, disabledHint }: ChatViewProps) {
  const isEmpty = messages.length === 0;
  const composerSlotRef = useRef<HTMLDivElement>(null);
  // Where the composer sat at the last commit, used as the FLIP origin. Only the
  // landing layout records it, because that is the only position it moves from.
  const landingTopRef = useRef<number | null>(null);
  const wasEmptyRef = useRef(isEmpty);

  // FLIP: the layout has already changed by the time this runs, so the composer
  // is offset back to where it used to be and animated to zero. Transforms keep
  // the whole move on the compositor, which matters because the message list is
  // mounting and streaming in the same frames.
  useLayoutEffect(() => {
    const slot = composerSlotRef.current;
    if (!slot) return;

    const wasEmpty = wasEmptyRef.current;
    if (wasEmpty === isEmpty) {
      // Steady state. Re-measuring on every streamed chunk would force a layout
      // for a value only the landing layout ever reads, so skip it there.
      if (isEmpty) landingTopRef.current = slot.getBoundingClientRect().top;
      return;
    }
    wasEmptyRef.current = isEmpty;

    const origin = isEmpty ? null : landingTopRef.current;
    if (origin === null) return;
    // The global reduced-motion rule only neutralises CSS animations; the Web
    // Animations API has to opt out by hand.
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;

    const deltaY = origin - slot.getBoundingClientRect().top;
    if (deltaY === 0) return;
    slot.animate(
      [{ transform: `translateY(${deltaY}px)` }, { transform: "translateY(0)" }],
      { duration: SLIDE_DURATION_MS, easing: SLIDE_EASING },
    );
  });

  return (
    <main className={`flex min-h-0 flex-1 flex-col bg-background ${isEmpty ? "overflow-y-auto" : ""}`}>
      {isEmpty ? (
        // `mt-auto` here and `mb-auto` on the composer slot split the free space
        // evenly, centring the pair. Auto margins collapse to 0 once the content
        // outgrows the pane, so a tall composer scrolls instead of being clipped.
        <div className="mt-auto w-full px-3 pt-10 sm:px-6">
          <div className="mx-auto w-full max-w-[760px]">
            <LandingHeading />
          </div>
        </div>
      ) : (
        <MessageList messages={messages} userName={userName} isResponding={isResponding} />
      )}

      <div
        ref={composerSlotRef}
        className={
          isEmpty
            ? "mb-auto w-full px-3 pb-10 sm:px-6"
            // Gradient fade so the thread dissolves under the composer instead of hard-clipping.
            : "shrink-0 bg-gradient-to-t from-background via-background to-transparent px-3 pb-4 pt-6 sm:px-5"
        }
      >
        <div className="mx-auto w-full max-w-[760px]">
          {error && <p role="alert" className="mb-2 px-1 text-xs text-destructive">{error}</p>}
          {/* Inset just past the composer's 16px corner radius, so the strip's edge
              clears the curve instead of sitting on it, and pulled down so the
              composer card overlaps the strip's lower padding. */}
          {contextBar && <div className="-mb-3 px-5">{contextBar}</div>}
          {/* The hint hangs off a wrapper because a disabled textarea swallows the
              pointer events a trigger needs. The wrapper stays mounted whether or not
              there is a hint: swapping it out would remount the composer and throw
              away whatever the user had already typed. Tracking the cursor keeps the
              bubble near the pointer, since the composer spans the whole pane. */}
          {/* Disabling the root rather than only withholding the content is what
              keeps a stale hover from surfacing later: the composer slides out from
              under the pointer when a thread opens, which leaves no pointerleave
              behind, so an enabled tooltip would still believe it is hovered and pop
              open the moment a hint reappears. */}
          <Tooltip trackCursorAxis="both" disabled={disabledHint === undefined}>
            <TooltipTrigger render={<div />}>
              <Composer autoFocus onSend={onSend} isResponding={isResponding} disabled={disabled} />
            </TooltipTrigger>
            <TooltipContent sideOffset={12}>{disabledHint}</TooltipContent>
          </Tooltip>
          {isEmpty && (
            <LandingSuggestions onSend={onSend} isResponding={isResponding} disabled={disabled} />
          )}
        </div>
      </div>
    </main>
  );
}
