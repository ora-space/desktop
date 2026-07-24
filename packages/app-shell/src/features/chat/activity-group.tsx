import { useState } from "react";
import {
  IconAlertTriangle,
  IconCheck,
  IconChevronDown,
  IconFiles,
  IconLoader2,
  IconRoute,
  IconSearch,
  IconSparkles,
  IconTimeline,
  IconWorld,
} from "@tabler/icons-react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@ora/ui";
import { useTranslation } from "react-i18next";
import type { ChatThought, ChatToolCall, ChatTurnStatus } from "@ora/chat";
import { ToolCallBlock } from "./tool-call-block";

export type ActivityItem = ChatThought | ChatToolCall;

interface ActivityGroupProps {
  items: ActivityItem[];
  turnStatus: ChatTurnStatus;
  isLatestActivity: boolean;
}

type ActivityStatus = "active" | "completed" | "failed";

interface ThoughtTimelineEntry {
  kind: "thought";
  id: string;
  thought: ChatThought;
}

interface ToolTimelineEntry {
  kind: "tools";
  id: string;
  tools: ChatToolCall[];
}

type TimelineEntry = ThoughtTimelineEntry | ToolTimelineEntry;

/** Condenses exploratory reasoning and file reads into one secondary timeline. */
export function ActivityGroup({ items, turnStatus, isLatestActivity }: ActivityGroupProps) {
  const status = activityStatus(items, turnStatus, isLatestActivity);
  const entries = groupTimelineEntries(items);
  const [disclosure, setDisclosure] = useState({ status, open: status !== "completed" });
  const latestItemId = entries.at(-1)?.id ?? null;
  const [selection, setSelection] = useState({
    followsLatest: status === "active",
    selectedId: status === "active" ? latestItemId : null,
  });
  if (disclosure.status !== status) {
    setDisclosure({ status, open: status !== "completed" });
  }
  const open = disclosure.open;
  const selectedId = selection.followsLatest && status === "active" ? latestItemId : selection.selectedId;

  const toggleItem = (itemId: string, nextOpen: boolean) => {
    setSelection({ followsLatest: false, selectedId: nextOpen ? itemId : null });
  };

  return (
    <Collapsible
      open={open}
      onOpenChange={(nextOpen) => setDisclosure({ status, open: nextOpen })}
      className={`relative overflow-hidden border-l-2 ${status === "active" ? "border-sky-500/70" : status === "failed" ? "border-destructive/70" : "border-border"}`}
    >
      <CollapsibleTrigger className="flex min-h-11 w-full items-center gap-2.5 rounded-r-sm px-3 py-1 text-left outline-none transition-colors duration-200 hover:bg-muted/25 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50">
        <IconTimeline className="size-4 shrink-0 text-sky-700 dark:text-sky-400" />
        <span className="min-w-0 flex-1">
          <span key={`${status}-${latestItemId}`} className="block truncate text-xs font-medium text-foreground animate-in fade-in slide-in-from-bottom-1 duration-200 motion-reduce:animate-none">
            <ActivityTitle status={status} items={items} />
          </span>
          <span className="mt-0.5 block truncate font-mono text-[10px] text-muted-foreground">
            <ActivityMetrics items={items} />
          </span>
        </span>
        <ActivityStatusIcon status={status} />
        <IconChevronDown className={`size-3.5 shrink-0 text-muted-foreground transition-transform duration-200 motion-reduce:transition-none ${open ? "rotate-180" : ""}`} />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <ol className="relative ml-5 border-l border-border/60 py-1 pl-2 pr-2">
          {entries.map((entry) => (
            <li key={entry.id} className="relative pl-3">
              <TimelineNodeMarker kind={entry.kind} selected={selectedId === entry.id} />
              {entry.kind === "thought" ? (
                <ThoughtTimelineItem
                  thought={entry.thought}
                  open={selectedId === entry.id}
                  onOpenChange={(nextOpen) => toggleItem(entry.id, nextOpen)}
                />
              ) : entry.tools.length === 1 ? (
                <ToolCallBlock
                  tool={entry.tools[0]}
                  appearance="timeline"
                  expanded={selectedId === entry.id}
                  onExpandedChange={(nextOpen) => toggleItem(entry.id, nextOpen)}
                />
              ) : (
                <ToolTimelineCluster
                  tools={entry.tools}
                  open={selectedId === entry.id}
                  onOpenChange={(nextOpen) => toggleItem(entry.id, nextOpen)}
                />
              )}
            </li>
          ))}
        </ol>
      </CollapsibleContent>
    </Collapsible>
  );
}

/** Groups only consecutive tools of the same kind so compression never rewrites chronology. */
function groupTimelineEntries(items: ActivityItem[]): TimelineEntry[] {
  const entries: TimelineEntry[] = [];
  let pendingTools: ChatToolCall[] = [];

  const flushTools = () => {
    if (pendingTools.length > 0) {
      entries.push({ kind: "tools", id: `tool-cluster-${pendingTools[0].id}`, tools: pendingTools });
    }
    pendingTools = [];
  };

  for (const item of items) {
    if (item.kind === "thought") {
      flushTools();
      entries.push({ kind: "thought", id: item.id, thought: item });
      continue;
    }
    if (pendingTools.length > 0 && pendingTools[0].toolKind !== item.toolKind) flushTools();
    pendingTools.push(item);
  }
  flushTools();
  return entries;
}

/** Names the live operation from structured tool data instead of provider-authored titles. */
function ActivityTitle({ status, items }: { status: ActivityStatus; items: ActivityItem[] }) {
  const { t } = useTranslation();
  if (status === "failed") return t("chat.activity.failed");
  const latestItem = items.at(-1);
  if (status === "completed") {
    const toolKinds = new Set(
      items
        .filter((item): item is ChatToolCall => item.kind === "toolCall")
        .map((tool) => tool.toolKind),
    );
    if (toolKinds.size === 0) return t("chat.activity.completed.analysis");
    if (toolKinds.size > 1) return t("chat.activity.completed.exploration");
    if (toolKinds.has("read")) return t("chat.activity.completed.read");
    if (toolKinds.has("search")) return t("chat.activity.completed.search");
    if (toolKinds.has("fetch")) return t("chat.activity.completed.fetch");
    return t("chat.activity.completed.tool");
  }
  if (latestItem === undefined || latestItem.kind === "thought") return t("chat.activity.active.analysis");

  switch (latestItem.toolKind) {
    case "read": {
      const path = latestItem.locations.at(-1)?.path;
      const target = path?.split(/[\\/]/).at(-1);
      return target === undefined
        ? t("chat.activity.active.read")
        : t("chat.activity.active.readTarget", { target });
    }
    case "search":
      return t("chat.activity.active.search");
    case "fetch":
      return t("chat.activity.active.fetch");
    case "edit":
    case "delete":
    case "move":
    case "execute":
    case "think":
    case "switch_mode":
    case "other":
    case undefined:
      return t("chat.activity.active.tool");
  }
}

/** Summarizes concrete operations so users can distinguish reads, searches, fetches, and reasoning. */
function ActivityMetrics({ items }: { items: ActivityItem[] }) {
  const { t } = useTranslation();
  const thoughts = items.filter((item) => item.kind === "thought").length;
  const tools = items.filter((item): item is ChatToolCall => item.kind === "toolCall");
  const readTools = tools.filter((tool) => tool.toolKind === "read");
  const paths = new Set(readTools.flatMap((tool) => tool.locations.map((location) => location.path)));
  const files = paths.size > 0 ? paths.size : readTools.length;
  const searches = tools.filter((tool) => tool.toolKind === "search").length;
  const fetches = tools.filter((tool) => tool.toolKind === "fetch").length;
  const metrics = [
    files > 0 ? t("chat.activity.metric.files", { count: files }) : null,
    searches > 0 ? t("chat.activity.metric.searches", { count: searches }) : null,
    fetches > 0 ? t("chat.activity.metric.fetches", { count: fetches }) : null,
    thoughts > 0 ? t("chat.activity.metric.thoughts", { count: thoughts }) : null,
  ].filter((metric): metric is string => metric !== null);
  return metrics.join(" · ");
}

/** Keeps one reasoning detail in the group's shared single-focus disclosure state. */
function ThoughtTimelineItem({
  thought,
  open,
  onOpenChange,
}: {
  thought: ChatThought;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  return (
    <Collapsible open={open} onOpenChange={onOpenChange}>
      <CollapsibleTrigger className={`flex min-h-11 w-full items-center gap-2 rounded-r-sm px-2 py-1.5 text-left text-xs outline-none transition-colors duration-200 hover:bg-muted/25 hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50 ${open ? "bg-muted/25 text-foreground" : "text-muted-foreground"}`}>
        <IconRoute className="size-4 shrink-0 text-violet-600 dark:text-violet-400" />
        <span className="shrink-0 font-medium">{t("chat.thought")}</span>
        <span className="min-w-0 flex-1 truncate opacity-80">{thought.content}</span>
        <IconChevronDown className={`size-3.5 shrink-0 transition-transform duration-200 motion-reduce:transition-none ${open ? "rotate-180" : ""}`} />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <p data-selectable className="ml-6 rounded-r-sm border-l-2 border-violet-500/50 bg-muted/20 px-3 py-2 text-xs leading-5 whitespace-pre-wrap text-muted-foreground">
          {thought.content}
        </p>
      </CollapsibleContent>
    </Collapsible>
  );
}

/** Reveals a batch summary before allowing access to each underlying tool result. */
function ToolTimelineCluster({
  tools,
  open,
  onOpenChange,
}: {
  tools: ChatToolCall[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation();
  const toolKind = tools[0]?.toolKind;
  const previews = clusterPreviewItems(tools);
  return (
    <Collapsible open={open} onOpenChange={onOpenChange}>
      <CollapsibleTrigger className={`flex min-h-11 w-full items-center gap-2 rounded-r-sm px-2 py-1 text-left text-xs outline-none transition-colors duration-200 hover:bg-muted/25 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50 ${open ? "bg-muted/25" : ""}`}>
        <ClusterIcon toolKind={toolKind} />
        <span className="min-w-0 flex-1">
          <span className="block truncate font-medium">{clusterTitle(toolKind, tools.length, t)}</span>
          {previews.length > 0 && <span className="mt-0.5 block truncate font-mono text-[10px] text-muted-foreground">{previews.join(" · ")}</span>}
        </span>
        <ActivityStatusIcon status={clusterStatus(tools)} />
        <IconChevronDown className={`size-3.5 shrink-0 text-muted-foreground transition-transform duration-200 motion-reduce:transition-none ${open ? "rotate-180" : ""}`} />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className="ml-4 border-l border-border/60 pl-2">
          {tools.map((tool) => <ToolCallBlock key={tool.id} tool={tool} appearance="embedded" />)}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}

/** Selects the same semantic icon family used by individual tool rows. */
function ClusterIcon({ toolKind }: { toolKind: ChatToolCall["toolKind"] }) {
  switch (toolKind) {
    case "read":
      return <IconFiles className="size-4 shrink-0 text-sky-600" />;
    case "search":
      return <IconSearch className="size-4 shrink-0 text-sky-600" />;
    case "fetch":
      return <IconWorld className="size-4 shrink-0 text-sky-600" />;
    case "edit":
    case "delete":
    case "move":
    case "execute":
    case "think":
    case "switch_mode":
    case "other":
    case undefined:
      return <IconSparkles className="size-4 shrink-0 text-muted-foreground" />;
  }
}

/** Produces a concise localized heading for one homogeneous tool batch. */
function clusterTitle(
  toolKind: ChatToolCall["toolKind"],
  count: number,
  t: (key: string, options?: { count: number }) => string,
): string {
  switch (toolKind) {
    case "read":
      return t("chat.activity.cluster.read", { count });
    case "search":
      return t("chat.activity.cluster.search", { count });
    case "fetch":
      return t("chat.activity.cluster.fetch", { count });
    case "edit":
    case "delete":
    case "move":
    case "execute":
    case "think":
    case "switch_mode":
    case "other":
    case undefined:
      return t("chat.activity.cluster.tool", { count });
  }
}

/** Uses paths where available and caps the preview so the batch stays one line. */
function clusterPreviewItems(tools: ChatToolCall[]): string[] {
  const labels = tools.map((tool) => {
    const path = tool.locations.at(-1)?.path;
    return path?.split(/[\\/]/).at(-1) ?? tool.title;
  });
  const visible = labels.slice(0, 3);
  if (labels.length > visible.length) visible.push(`+${labels.length - visible.length}`);
  return visible;
}

/** Prevents completed calls from masking a failed or still-running item in the batch. */
function clusterStatus(tools: ChatToolCall[]): ActivityStatus {
  if (tools.some((tool) => tool.status === "failed")) return "failed";
  if (tools.some((tool) => tool.status === "pending" || tool.status === "in_progress")) return "active";
  return "completed";
}

/** Gives the focused node a stable locator without changing the timeline's geometry. */
function TimelineNodeMarker({ kind, selected }: { kind: TimelineEntry["kind"]; selected: boolean }) {
  return (
    <span className="absolute -left-[15px] top-[17px] flex size-2 items-center justify-center bg-background" aria-hidden="true">
      <span className={`transition-all duration-200 ${kind === "thought" ? "h-2 w-1 rounded-full" : "size-1.5 rounded-full"} ${selected ? "bg-foreground ring-2 ring-background outline outline-1 outline-foreground/25" : "bg-border"}`} />
    </span>
  );
}

/** Resolves activity lifecycle without letting completed reads hide a live final thought. */
function activityStatus(items: ActivityItem[], turnStatus: ChatTurnStatus, isLatestActivity: boolean): ActivityStatus {
  const tools = items.filter((item): item is ChatToolCall => item.kind === "toolCall");
  if (turnStatus === "failed" || tools.some((tool) => tool.status === "failed")) return "failed";
  if (
    tools.some((tool) => tool.status === "pending" || tool.status === "in_progress")
    || (turnStatus === "streaming" && isLatestActivity)
  ) return "active";
  return "completed";
}

/** Communicates status without repeating a full text badge beside the descriptive heading. */
function ActivityStatusIcon({ status }: { status: ActivityStatus }) {
  const { t } = useTranslation();
  switch (status) {
    case "active":
      return <IconLoader2 className="size-3.5 shrink-0 animate-spin text-sky-600 motion-reduce:animate-none" aria-label={t("chat.toolRunning")} />;
    case "completed":
      return <IconCheck className="size-3.5 shrink-0 text-emerald-600" aria-label={t("chat.toolCompleted")} />;
    case "failed":
      return <IconAlertTriangle className="size-3.5 shrink-0 text-destructive" aria-label={t("chat.toolFailed")} />;
  }
}
