import { useState } from "react";
import { diffLines } from "diff";
import {
  IconChevronDown,
  IconFileDiff,
  IconFiles,
  IconTerminal2,
} from "@tabler/icons-react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@ora/ui";
import { useTranslation } from "react-i18next";
import type { ChatToolCall } from "@ora/chat";
import type { acp } from "@ora/contracts";
import { ToolCallBlock, ToolStatus } from "./tool-call-block";
import type { ToolCallGroupKind } from "./tool-call-group-kind";

interface ToolCallGroupProps {
  kind: ToolCallGroupKind;
  tools: ChatToolCall[];
}

/** Condenses related tool calls while preserving every original result behind one disclosure. */
export function ToolCallGroup({ kind, tools }: ToolCallGroupProps) {
  const status = groupStatus(tools);
  const [disclosure, setDisclosure] = useState({ status, open: status !== "completed" });
  if (disclosure.status !== status) {
    setDisclosure({ status, open: status !== "completed" });
  }
  const open = disclosure.open;
  const { t } = useTranslation();
  const paths = uniquePaths(tools);
  const previewItems = kind === "commands" ? tools.map((tool) => tool.title) : paths.map(fileName);
  const visibleItems = previewItems.slice(0, 3);
  const summaryCount = kind === "commands" || paths.length === 0 ? tools.length : paths.length;
  const diffTotals = kind === "changes" ? countChanges(tools) : null;
  const phase = status === "completed" ? "completed" : status === "failed" ? "failed" : "active";

  return (
    <Collapsible
      open={open}
      onOpenChange={(nextOpen) => setDisclosure({ status, open: nextOpen })}
      className="overflow-hidden rounded-md border border-border/80 bg-muted/15"
    >
      <CollapsibleTrigger className="flex min-h-11 w-full items-center gap-2.5 px-3 py-2 text-left outline-none transition-colors hover:bg-muted/35 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring">
        <GroupIcon kind={kind} />
        <span className="min-w-0 flex-1">
          <span className="flex min-w-0 items-center gap-2">
            <span className="truncate text-xs font-medium">
              {t(`chat.toolGroup.${kind}.${phase}`, { count: summaryCount })}
            </span>
            {diffTotals !== null && (diffTotals.additions > 0 || diffTotals.deletions > 0) && (
              <span className="flex shrink-0 gap-1 font-mono text-[10px]" aria-label={t("chat.toolGroup.changeStats", diffTotals)}>
                <span className="text-emerald-600">+{diffTotals.additions}</span>
                <span className="text-red-600">-{diffTotals.deletions}</span>
              </span>
            )}
          </span>
          {visibleItems.length > 0 && (
            <span className="mt-0.5 block truncate font-mono text-[10px] text-muted-foreground">
              {visibleItems.join(", ")}
              {previewItems.length > visibleItems.length ? ` +${previewItems.length - visibleItems.length}` : ""}
            </span>
          )}
        </span>
        <ToolStatus status={status} />
        <IconChevronDown className={`size-3.5 shrink-0 text-muted-foreground transition-transform motion-reduce:transition-none ${open ? "rotate-180" : ""}`} />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className="divide-y divide-border/60 border-t border-border/70">
          {tools.map((tool) => <ToolCallBlock key={tool.id} tool={tool} appearance="embedded" />)}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}

/** Derives one status without allowing completed calls to mask active or failed work. */
function groupStatus(tools: ChatToolCall[]): acp.ToolCallStatus | undefined {
  if (tools.some((tool) => tool.status === "failed")) return "failed";
  if (tools.some((tool) => tool.status === "in_progress")) return "in_progress";
  if (tools.some((tool) => tool.status === "pending")) return "pending";
  if (tools.every((tool) => tool.status === "completed")) return "completed";
  return undefined;
}

/** Selects an icon and restrained accent that communicate each activity category. */
function GroupIcon({ kind }: { kind: ToolCallGroupKind }) {
  switch (kind) {
    case "exploration":
      return <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-sky-500/10 text-sky-700 dark:text-sky-400"><IconFiles className="size-4" /></span>;
    case "changes":
      return <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-violet-500/10 text-violet-700 dark:text-violet-400"><IconFileDiff className="size-4" /></span>;
    case "commands":
      return <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-amber-500/10 text-amber-700 dark:text-amber-400"><IconTerminal2 className="size-4" /></span>;
  }
}

/** Removes repeated locations so a tool touching one path multiple times does not inflate the summary. */
function uniquePaths(tools: ChatToolCall[]): string[] {
  return [...new Set(tools.flatMap((tool) => tool.locations.map((location) => location.path)))];
}

/** Keeps the summary scannable while full paths remain available after expansion. */
function fileName(path: string): string {
  return path.split(/[\\/]/).at(-1) ?? path;
}

/** Counts line additions and deletions across all structured diffs in one change group. */
function countChanges(tools: ChatToolCall[]): { additions: number; deletions: number } {
  let additions = 0;
  let deletions = 0;
  for (const tool of tools) {
    for (const content of tool.content) {
      if (content.type !== "diff") continue;
      for (const part of diffLines(content.oldText ?? "", content.newText)) {
        const lineCount = part.value.endsWith("\n") ? part.count ?? 0 : (part.count ?? 1);
        if (part.added) additions += lineCount;
        if (part.removed) deletions += lineCount;
      }
    }
  }
  return { additions, deletions };
}
