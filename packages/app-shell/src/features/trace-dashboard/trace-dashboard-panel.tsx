import { useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "@ora/ui";
import { IconX } from "@tabler/icons-react";
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
 * Right-side docked panel (Cursor/ChatGPT-style) showing the trace dashboard for
 * the current session. The dashboard is the only surface today; review and
 * terminal are out of scope for now, so the panel renders the dashboard body
 * directly instead of a tab switcher that would only show "coming soon".
 * Its left edge is a pointer-drag handle so the dashboard can be widened to fit
 * its charts, and pointer capture keeps the resize alive over the iframe.
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

  if (!open) return null;

  return (
    <aside
      data-slot="trace-dashboard-panel"
      className="relative z-30 flex h-full shrink-0 flex-col border-l bg-popover text-sm text-popover-foreground shadow-lg"
      style={{ width: clamp(width) }}
      role="complementary"
      aria-label={t("dashboard.title")}
    >
      <PanelResizeHandle
        onResize={(delta, startWidth) => setWidth(clamp(startWidth - delta))}
        aria-label={t("dashboard.resize")}
      />
      <div className="flex items-center justify-between gap-2 px-4 pt-3 pb-2">
        <h2 className="text-sm font-medium tracking-[-0.01em]">{t("dashboard.title")}</h2>
        <button
          type="button"
          className="inline-flex size-6 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-foreground/5 hover:text-foreground"
          aria-label={t("dashboard.close")}
          onClick={() => setOpen(false)}
        >
          <IconX className="size-4" />
        </button>
      </div>
      <div className="h-full min-h-0 flex-1 px-4 pb-4">
        <DashboardTabBody
          sessionId={sessionId}
          isLoading={isLoading}
          error={error}
          endpoint={endpoint}
        />
      </div>
    </aside>
  );
}

/**
 * Draggable left edge of the panel. Pointer capture keeps receiving move events
 * even when the cursor drifts over the iframe (which would otherwise swallow
 * them), so the resize never "sticks" mid-drag over the embedded dashboard.
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
      onPointerDown={(event) => {
        event.preventDefault();
        event.currentTarget.setPointerCapture?.(event.pointerId);
        startRef.current = {
          clientX: event.clientX,
          // The handle's parent <aside> carries the live width; capture it at start.
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
      {...props}
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
