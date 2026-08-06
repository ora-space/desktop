import { useTranslation } from "react-i18next";
import {
  IconAlertTriangle,
  IconBan,
  IconCheck,
  IconLoader2,
  IconMinus,
  IconX,
} from "@tabler/icons-react";
import { Badge, cn } from "@ora/ui";
import { isNodeWorking, runStatusTone } from "./run-status-style";
import type {
  GraphWorkflowNodeStatus,
  GraphWorkflowRunStatus,
} from "@ora/workflow-runtime";
import "./theater-motion.css";

type Status = GraphWorkflowRunStatus | GraphWorkflowNodeStatus;

type TerminalStatus = Extract<
  Status,
  "succeeded" | "failed" | "partial_failed" | "cancelled" | "skipped"
>;

function isTerminal(status: Status): status is TerminalStatus {
  return status === "succeeded"
    || status === "failed"
    || status === "partial_failed"
    || status === "cancelled"
    || status === "skipped";
}

const ICON_BOX = "size-3.5";
const ICON_GLYPH = "size-2.5";

/**
 * Status mark—pick exactly one language per surface:
 * - `live`: spinner (working cue on the focused card only)
 * - terminal + not quiet: check / x glyph
 * - otherwise: pure color dot (path, header, inspector, idle/pending)
 */
export function RunStatusMark({
  status,
  live = false,
  quiet = false,
  className,
}: {
  status: Status;
  live?: boolean;
  quiet?: boolean;
  className?: string;
}) {
  const tone = runStatusTone(status);

  if (live && isNodeWorking(status)) {
    return (
      <span
        className={cn(
          "inline-flex shrink-0 items-center justify-center rounded-full text-white",
          ICON_BOX,
          status === "awaiting_input" ? "bg-amber-500" : "bg-sky-500",
          className,
        )}
        aria-hidden
      >
        <IconLoader2
          className={cn(ICON_GLYPH, "motion-safe:animate-spin")}
          stroke={2.5}
        />
      </span>
    );
  }

  if (!quiet && isTerminal(status)) {
    return (
      <span
        className={cn(
          "inline-flex shrink-0 items-center justify-center rounded-full text-white",
          ICON_BOX,
          terminalSurface(status),
          className,
        )}
        aria-hidden
      >
        <TerminalGlyph status={status} className={ICON_GLYPH} />
      </span>
    );
  }

  // Quiet path/header: partial_failed uses a tiny triangle so it is not identical to failed.
  if (quiet && status === "partial_failed") {
    return (
      <span
        className={cn(
          "inline-flex size-2.5 shrink-0 items-center justify-center text-rose-500",
          className,
        )}
        aria-hidden
      >
        <IconAlertTriangle className="size-2.5" stroke={2.5} />
      </span>
    );
  }

  return (
    <span
      className={cn("inline-flex size-1.5 shrink-0 rounded-full", tone.dot, className)}
      aria-hidden
    />
  );
}

/** One mark + label. Pass `live` only on the card that owns the working cue. */
export function RunStatusBadge({
  status,
  live = false,
  quiet = false,
  className,
}: {
  status: Status;
  live?: boolean;
  quiet?: boolean;
  className?: string;
}) {
  const { t } = useTranslation();
  const tone = runStatusTone(status);
  return (
    <Badge
      variant="outline"
      className={cn(
        "gap-1.5 border transition-colors duration-200",
        tone.badge,
        className,
      )}
    >
      <RunStatusMark status={status} live={live} quiet={quiet} />
      {t(tone.labelKey)}
    </Badge>
  );
}

function TerminalGlyph({
  status,
  className,
}: {
  status: TerminalStatus;
  className: string;
}) {
  switch (status) {
    case "succeeded":
      return <IconCheck className={className} stroke={3} />;
    case "failed":
      return <IconX className={className} stroke={3} />;
    case "partial_failed":
      return <IconAlertTriangle className={className} stroke={2.5} />;
    case "cancelled":
      return <IconBan className={className} stroke={2.5} />;
    case "skipped":
      return <IconMinus className={className} stroke={2.5} />;
  }
}

function terminalSurface(status: Status): string {
  switch (status) {
    case "succeeded":
      return "bg-emerald-500";
    case "failed":
    case "partial_failed":
      return "bg-rose-500";
    case "cancelled":
      return "bg-zinc-400 dark:bg-zinc-500";
    case "skipped":
      return "bg-muted text-muted-foreground";
    default:
      return "bg-muted text-muted-foreground";
  }
}
