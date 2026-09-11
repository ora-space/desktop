import { useState } from "react";
import {
  IconAlertTriangle,
  IconBan,
  IconCheck,
  IconChevronDown,
  IconListDetails,
} from "@tabler/icons-react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@ora/ui";
import { useTranslation } from "react-i18next";
import type { ChatToolCall, ChatTurnStatus } from "@ora/chat";
import { ActivityGroup } from "./activity-group";
import { PlanBlock } from "./plan-block";
import { ToolCallBlock } from "./tool-call-block";
import { ToolCallGroup } from "./tool-call-group";
import type {
  ActivityPhaseItem,
  NonTextDisplayItem,
} from "./turn-item-grouping";

interface ActivityPhaseProps {
  phase: ActivityPhaseItem;
  turnStatus: ChatTurnStatus;
  isLatestActivity: boolean;
}

type PhaseStatus = "completed" | "cancelled" | "failed";

/** Renders one turn's non-text activity: live and expanded while streaming, one collapsed summary once it ends. */
export function ActivityPhase({
  phase,
  turnStatus,
  isLatestActivity,
}: ActivityPhaseProps) {
  // A single block already collapses itself once settled; only distinct block types sharing
  // one phase need an outer disclosure to consolidate them into one summary.
  if (phase.live || phase.items.length <= 1) {
    return (
      <>
        {phase.items.map((item, index) => (
          <NonTextItemView
            key={item.id}
            item={item}
            turnStatus={turnStatus}
            isLatestActivity={
              isLatestActivity && index === phase.items.length - 1
            }
          />
        ))}
      </>
    );
  }
  return <CollapsedActivityPhase phase={phase} turnStatus={turnStatus} />;
}

/** Wraps a finished activity phase in one disclosure so unrelated block types no longer stay expanded side by side. */
function CollapsedActivityPhase({
  phase,
  turnStatus,
}: {
  phase: ActivityPhaseItem;
  turnStatus: ChatTurnStatus;
}) {
  const { t } = useTranslation();
  const status = phaseStatus(phase.items, turnStatus);
  const [open, setOpen] = useState(false);
  const count = countOperations(phase.items);

  return (
    <Collapsible
      open={open}
      onOpenChange={setOpen}
      className={`overflow-hidden border-l-2 ${status === "failed" ? "border-destructive/70" : "border-border"}`}
    >
      <CollapsibleTrigger className="flex min-h-11 w-full items-center gap-2.5 rounded-r-sm px-3 py-1.5 text-left outline-none transition-colors duration-200 hover:bg-muted/25 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50">
        <IconListDetails className="size-4 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-xs font-medium text-foreground">
            {t(`chat.activityPhase.title.${status}`)}
          </span>
          <span className="mt-0.5 block truncate font-mono text-[10px] text-muted-foreground">
            {t("chat.activityPhase.metric.operations", { count })}
          </span>
        </span>
        <PhaseStatusIcon status={status} />
        <IconChevronDown
          className={`size-3.5 shrink-0 text-muted-foreground transition-transform duration-200 motion-reduce:transition-none ${open ? "rotate-180" : ""}`}
        />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className="ml-5 space-y-2 border-l border-border/60 py-1 pl-2">
          {phase.items.map((item) => (
            <NonTextItemView
              key={item.id}
              item={item}
              turnStatus={turnStatus}
              isLatestActivity={false}
            />
          ))}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}

/** Dispatches one non-text turn item to its existing dedicated renderer. */
export function NonTextItemView({
  item,
  turnStatus,
  isLatestActivity,
}: {
  item: NonTextDisplayItem;
  turnStatus: ChatTurnStatus;
  isLatestActivity: boolean;
}) {
  switch (item.kind) {
    case "activityGroup":
      return (
        <ActivityGroup
          items={item.items}
          turnStatus={turnStatus}
          isLatestActivity={isLatestActivity}
        />
      );
    case "plan":
      return <PlanBlock plan={item} />;
    case "toolCall":
      return <ToolCallBlock tool={item} />;
    case "toolGroup":
      return <ToolCallGroup kind={item.groupKind} tools={item.tools} />;
  }
}

/** Resolves one status for the whole phase without letting successful items mask a failure or cancellation. */
function phaseStatus(
  items: NonTextDisplayItem[],
  turnStatus: ChatTurnStatus,
): PhaseStatus {
  const tools = collectToolCalls(items);
  if (turnStatus === "failed" || tools.some((tool) => tool.status === "failed"))
    return "failed";
  if (
    turnStatus === "cancelled" ||
    tools.some((tool) => tool.status === "cancelled")
  )
    return "cancelled";
  return "completed";
}

/** Flattens every leaf tool call out of activity groups and tool groups for status and count checks. */
function collectToolCalls(items: NonTextDisplayItem[]): ChatToolCall[] {
  return items.flatMap((item) => {
    if (item.kind === "toolCall") return [item];
    if (item.kind === "toolGroup") return item.tools;
    if (item.kind === "activityGroup")
      return item.items.filter(
        (entry): entry is ChatToolCall => entry.kind === "toolCall",
      );
    return [];
  });
}

/** Counts every reasoning step and tool call folded into this phase for the collapsed summary. */
function countOperations(items: NonTextDisplayItem[]): number {
  return items.reduce((total, item) => {
    if (item.kind === "activityGroup") return total + item.items.length;
    if (item.kind === "toolGroup") return total + item.tools.length;
    return total + 1;
  }, 0);
}

function PhaseStatusIcon({ status }: { status: PhaseStatus }) {
  const { t } = useTranslation();
  switch (status) {
    case "completed":
      return (
        <IconCheck
          className="size-3.5 shrink-0 text-emerald-600"
          aria-label={t("chat.toolCompleted")}
        />
      );
    case "cancelled":
      return (
        <IconBan
          className="size-3.5 shrink-0 text-muted-foreground"
          aria-label={t("chat.toolCancelled")}
        />
      );
    case "failed":
      return (
        <IconAlertTriangle
          className="size-3.5 shrink-0 text-destructive"
          aria-label={t("chat.toolFailed")}
        />
      );
  }
}
