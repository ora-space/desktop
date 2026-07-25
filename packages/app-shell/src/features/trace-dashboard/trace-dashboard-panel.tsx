import { useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { cn, Sheet, SheetContent, SheetHeader, SheetTitle } from "@ora/ui";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useDashboardEndpoint } from "./use-dashboard-endpoint";
import type { DashboardResolver } from "./types";

interface TraceDashboardPanelProps {
  /** Injected by the Desktop app via Tauri invoke; null in non-Desktop builds/tests. */
  resolveDashboardUrl: DashboardResolver | null;
}

// Sizing bounds for the docked panel; the handle clamps live drags to this range
// and reacts to nothing else, so a tug past the min/max never breaks the layout.
const MIN_PANEL_WIDTH = 420;
const MAX_PANEL_WIDTH = 1400;

/**
 * Right-side overlay panel (codex/ChatGPT-style) showing the trace dashboard for
 * the current session. It uses the shared `Sheet` primitive so the slide-in +
 * fade animation, the click-the-backdrop-to-close behavior, and the top-right
 * close button all come for free and stay consistent with the rest of the shell's
 * dialogs. The dashboard is the only surface today (review/terminal out of
 * scope), so the panel renders the dashboard body directly.
 *
 * Inside the sheet, the left edge is still a pointer-drag handle so the panel can
 * be widened to fit the dashboard's charts; the persisted pixel width overrides
 * the sheet's default proportional width. Closing the sheet unmounts the iframe,
 * so reopening re-resolves the trace (the Streamlit server stays running, so this
 * is cheap).
 */
export function TraceDashboardPanel({ resolveDashboardUrl }: TraceDashboardPanelProps) {
  const { t } = useTranslation();
  const open = useUiStore((s) => s.dashboardOpen);
  const setOpen = useUiStore((s) => s.setDashboardOpen);
  const width = useUiStore((s) => s.dashboardWidth);
  const setWidth = useUiStore((s) => s.setDashboardWidth);
  const sessionId = useWorkspaceSelectionStore((s) => s.selection.sessionId);

  const { endpoint, isLoading, error } = useDashboardEndpoint(
    sessionId,
    resolveDashboardUrl,
    open,
  );

  // Clamp the persisted width so a value saved from a larger window does not
  // render as a too-wide panel after the viewport shrank.
  const clamp = useCallback(
    (candidate: number) =>
      Math.min(
        MAX_PANEL_WIDTH,
        Math.max(MIN_PANEL_WIDTH, Math.round(candidate)),
      ),
    [],
  );

  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetContent
        side="right"
        showCloseButton
        // The sheet defaults to a proportional w-3/4 capped by sm:max-w-sm; override
        // both so the persisted pixel width (dragged by the handle) actually applies.
        // Inline width/maxWidth beats the class, but max-width must be neutralized
        // too or the data-[side=right]:sm:max-w-sm variant caps the panel at 24rem.
        className="w-auto gap-0 data-[side=right]:sm:max-w-none"
        style={{ width: clamp(width), maxWidth: "none" }}
        aria-label={t("dashboard.title")}
      >
        <PanelResizeHandle
          onResize={(delta, startWidth) => setWidth(clamp(startWidth - delta))}
          aria-label={t("dashboard.resize")}
        />
        <SheetHeader className="gap-0.5 pr-10">
          <SheetTitle>{t("dashboard.title")}</SheetTitle>
        </SheetHeader>
        <div className="h-full min-h-0 flex-1 px-4 pb-4">
          <DashboardTabBody
            sessionId={sessionId}
            isLoading={isLoading}
            error={error}
            endpoint={endpoint}
          />
        </div>
      </SheetContent>
    </Sheet>
  );
}

/**
 * Draggable left edge of the panel. Pointer capture keeps receiving move events
 * even when the cursor drifts onto the backdrop (which would otherwise swallow
 * them and close the sheet), so the resize never "sticks" mid-drag.
 */
function PanelResizeHandle({
  onResize,
  ...props
}: {
  onResize: (deltaFromStart: number, startWidth: number) => void;
} & React.HTMLAttributes<HTMLDivElement>) {
  const startRef = useRef<{ clientX: number; width: number } | null>(null);
  return (
    <div
      data-slot="trace-dashboard-resize-handle"
      className={cn(
        "absolute inset-y-0 left-0 z-40 flex w-1 cursor-ew-resize items-center justify-center",
        // A faint accent on hover/active signals the handle is interactive.
        "bg-transparent transition-colors hover:bg-primary/30 active:bg-primary/50",
        // Make the grab target wider than the 1px visual so it is easy to hit.
        "before:absolute before:inset-y-0 before:-left-1 before:w-2",
      )}
      {...props}
      onPointerDown={(event) => {
        event.preventDefault();
        event.currentTarget.setPointerCapture?.(event.pointerId);
        startRef.current = {
          clientX: event.clientX,
          // The handle's parent (SheetContent) carries the live width; capture it at start.
          width: event.currentTarget.parentElement?.getBoundingClientRect().width ?? 0,
        };
      }}
      onPointerMove={(event) => {
        const start = startRef.current;
        if (!start) return;
        onResize(event.clientX - start.clientX, start.width);
      }}
      onPointerUp={(event) => {
        startRef.current = null;
        event.currentTarget.releasePointerCapture?.(event.pointerId);
      }}
    />
  );
}

/** Renders the iframe or the right "not ready" state for the current session. */
function DashboardTabBody({
  sessionId,
  isLoading,
  error,
  endpoint,
}: {
  sessionId: string | null;
  isLoading: boolean;
  error: string | null;
  endpoint: { url: string; serverReachable: boolean } | null;
}) {
  const { t } = useTranslation();

  if (!sessionId) {
    return <StatusLine>{t("dashboard.noSession")}</StatusLine>;
  }
  if (isLoading) {
    return <StatusLine>{t("dashboard.resolving")}</StatusLine>;
  }
  if (error || !endpoint) {
    return <StatusLine>{t("dashboard.resolveError")}</StatusLine>;
  }
  if (!endpoint.serverReachable) {
    return <StatusLine>{t("dashboard.serverUnreachable")}</StatusLine>;
  }
  return (
    <iframe
      title={t("dashboard.tab.dashboard")}
      src={endpoint.url}
      className="h-full w-full rounded-md border bg-background"
      // The dashboard is a same-loopback HTTP server the user runs; allow scripts,
      // forms, and same-origin storage without forcing fullscreen or payment.
      sandbox="allow-scripts allow-same-origin allow-forms allow-popups"
      // Streamlit owns the iframe document; keep it opaque to the host origin.
      referrerPolicy="no-referrer"
    />
  );
}

/** A single muted line used for not-ready states. */
function StatusLine({ children }: { children: React.ReactNode }) {
  return <p className="py-6 text-sm text-muted-foreground">{children}</p>;
}
