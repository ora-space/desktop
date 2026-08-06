import type { GraphWorkflowNodeStatus, GraphWorkflowRunStatus } from "@ora/workflow-runtime";

/** Node is actively executing or blocked on HITL—the only spinner-worthy states. */
export function isNodeWorking(status: GraphWorkflowRunStatus | GraphWorkflowNodeStatus): boolean {
  return status === "running" || status === "awaiting_input";
}

/** Run-level terminal statuses. */
export function isTerminalRunStatus(status: GraphWorkflowRunStatus): boolean {
  return (
    status === "succeeded"
    || status === "failed"
    || status === "cancelled"
  );
}

/** Shared run/node status chrome —color is never the only signal (dot + label + tone). */
export function runStatusTone(status: GraphWorkflowRunStatus | GraphWorkflowNodeStatus): {
  dot: string;
  ring: string;
  badge: string;
  labelKey: string;
} {
  switch (status) {
    case "running":
      return {
        dot: "bg-sky-500",
        ring: "border-sky-500/50 ring-sky-500/20",
        badge: "border-sky-500/30 bg-sky-500/10 text-sky-800 dark:text-sky-300",
        labelKey: "workflowRun.status.running",
      };
    case "awaiting_input":
      // Amber matches HITL “must handle—chrome on the dock and stage card.
      return {
        dot: "bg-amber-500",
        ring: "border-amber-500/50 ring-amber-500/20",
        badge: "border-amber-500/35 bg-amber-500/10 text-amber-900 dark:text-amber-200",
        labelKey: "workflowRun.status.awaiting_input",
      };
    case "succeeded":
      return {
        dot: "bg-emerald-500",
        ring: "border-emerald-500/45 ring-emerald-500/15",
        badge: "border-emerald-500/30 bg-emerald-500/10 text-emerald-800 dark:text-emerald-300",
        labelKey: "workflowRun.status.succeeded",
      };
    case "failed":
      return {
        dot: "bg-rose-500",
        ring: "border-rose-500/45 ring-rose-500/15",
        badge: "border-rose-500/30 bg-rose-500/10 text-rose-800 dark:text-rose-300",
        labelKey: "workflowRun.status.failed",
      };
    case "cancelled":
      return {
        dot: "bg-zinc-400",
        ring: "border-zinc-400/45 ring-zinc-400/10",
        badge: "border-zinc-400/30 bg-zinc-500/10 text-zinc-700 dark:text-zinc-300",
        labelKey: "workflowRun.status.cancelled",
      };
    case "pending":
      return {
        dot: "bg-amber-400",
        ring: "border-amber-400/40 ring-amber-400/10",
        badge: "border-amber-400/30 bg-amber-500/10 text-amber-900 dark:text-amber-200",
        labelKey: "workflowRun.status.pending",
      };
    case "idle":
      return {
        dot: "bg-muted-foreground/35",
        ring: "border-border ring-transparent",
        badge: "border-border bg-muted/60 text-muted-foreground",
        labelKey: "workflowRun.nodeStatus.idle",
      };
  }
}
