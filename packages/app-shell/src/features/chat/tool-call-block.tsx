import { useState } from "react";
import {
  IconAlertTriangle,
  IconCheck,
  IconChevronDown,
  IconCode,
  IconFileDiff,
  IconFileText,
  IconLoader2,
  IconArrowsExchange,
  IconArrowsMove,
  IconSearch,
  IconTerminal2,
  IconTool,
  IconTrash,
  IconWorld,
} from "@tabler/icons-react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@ora/ui";
import { useTranslation } from "react-i18next";
import type { ChatToolCall } from "@ora/chat";
import type { acp } from "@ora/contracts";
import { DiffView } from "./diff-view";

interface ToolCallBlockProps {
  tool: ChatToolCall;
  appearance?: "standalone" | "embedded" | "timeline";
  expanded?: boolean;
  onExpandedChange?: (expanded: boolean) => void;
}

/** Displays one tool lifecycle and keeps failed or active work visible by default. */
export function ToolCallBlock({
  tool,
  appearance = "standalone",
  expanded,
  onExpandedChange,
}: ToolCallBlockProps) {
  const [disclosure, setDisclosure] = useState({ status: tool.status, open: tool.status !== "completed" });
  if (expanded === undefined && disclosure.status !== tool.status) {
    setDisclosure({
      status: tool.status,
      open: tool.status !== "completed",
    });
  }
  const open = expanded ?? disclosure.open;
  const setOpen = (nextOpen: boolean) => {
    if (onExpandedChange !== undefined) onExpandedChange(nextOpen);
    else setDisclosure({ status: tool.status, open: nextOpen });
  };
  const hasDetails = tool.locations.length > 0 || tool.content.length > 0 || tool.rawInput !== undefined || tool.rawOutput !== undefined;
  const compactStatus = appearance === "timeline";
  const displayTitle = toolDisplayTitle(tool);
  const standaloneRail = appearance === "standalone" ? toolRailClass(tool.toolKind) : "";
  const summary = (
    <>
      <ToolKindIcon kind={tool.toolKind} />
      <ToolKindLabel kind={tool.toolKind} />
      {displayTitle !== null && <span className="min-w-0 flex-1 truncate font-medium" title={tool.locations.at(-1)?.path ?? tool.title}>{displayTitle}</span>}
      {displayTitle === null && <span className="min-w-0 flex-1" />}
      <ToolStatus status={tool.status} compact={compactStatus} />
      {hasDetails && <IconChevronDown className={`size-3.5 shrink-0 text-muted-foreground transition-transform motion-reduce:transition-none ${open ? "rotate-180" : ""}`} />}
    </>
  );

  if (!hasDetails) {
    return (
      <div className={`flex min-h-11 w-full items-center gap-2 px-3 py-1.5 text-xs ${standaloneRail}`}>
        {summary}
      </div>
    );
  }

  return (
    <Collapsible
      open={open}
      onOpenChange={setOpen}
      className={`overflow-hidden bg-transparent ${standaloneRail}`}
    >
      <CollapsibleTrigger className={`flex min-h-11 w-full items-center gap-2 rounded-r-sm px-3 py-1.5 text-left text-xs outline-none transition-colors duration-200 hover:bg-muted/25 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/50 ${appearance === "timeline" && open ? "bg-muted/25" : ""}`}>
        {summary}
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className={`space-y-3 px-3 py-2.5 ${appearance !== "standalone" ? "ml-4 pl-6" : "pl-9"}`}>
          {tool.locations.length > 0 && (
            <div className="space-y-1">
              {tool.locations.map((location) => (
                <code data-selectable key={`${location.path}:${location.line ?? ""}`} className="block max-w-full truncate text-[11px] text-sky-700 dark:text-sky-400" title={location.path}>
                  {location.path}{location.line === undefined || location.line === null ? "" : `:${location.line}`}
                </code>
              ))}
            </div>
          )}
          {tool.content.map((content, index) => (
            <ToolContent key={index} content={content} />
          ))}
          {(tool.rawInput !== undefined || tool.rawOutput !== undefined) && (
            <RawData input={tool.rawInput} output={tool.rawOutput} />
          )}
        </div>
      </CollapsibleContent>
    </Collapsible>
  );
}

/** Mirrors each tool category's icon accent on the standalone content rail. */
function toolRailClass(kind: acp.ToolKind | undefined): string {
  switch (kind) {
    case "read":
    case "search":
    case "fetch":
      return "border-l-2 border-sky-500/60";
    case "edit":
    case "move":
    case "think":
      return "border-l-2 border-violet-500/60";
    case "delete":
      return "border-l-2 border-destructive/70";
    case "execute":
      return "border-l-2 border-amber-500/60";
    case "switch_mode":
    case "other":
    case undefined:
      return "border-l-2 border-border";
  }
}

/** Prefers structured paths and removes provider titles that merely repeat the action kind. */
function toolDisplayTitle(tool: ChatToolCall): string | null {
  const path = tool.locations.length === 1 ? tool.locations[0].path : undefined;
  if (path !== undefined && (tool.toolKind === "read" || tool.toolKind === "edit" || tool.toolKind === "delete" || tool.toolKind === "move")) {
    return path.split(/[\\/]/).at(-1) ?? path;
  }
  const normalizedTitle = tool.title.trim().toLowerCase();
  const redundantTitles = new Set(["read", "search", "fetch", "edit", "execute", "think", "tool"]);
  return redundantTitles.has(normalizedTitle) ? null : tool.title;
}

/** Adds a localized action verb so tool meaning never depends on color or icon recognition. */
function ToolKindLabel({ kind }: { kind: acp.ToolKind | undefined }) {
  const { t } = useTranslation();
  const key = (() => {
    switch (kind) {
      case "read":
        return "chat.toolKind.read";
      case "edit":
        return "chat.toolKind.edit";
      case "delete":
        return "chat.toolKind.delete";
      case "move":
        return "chat.toolKind.move";
      case "search":
        return "chat.toolKind.search";
      case "execute":
        return "chat.toolKind.execute";
      case "think":
        return "chat.toolKind.think";
      case "fetch":
        return "chat.toolKind.fetch";
      case "switch_mode":
        return "chat.toolKind.switchMode";
      case "other":
      case undefined:
        return "chat.toolKind.other";
    }
  })();
  return <span className="shrink-0 text-[10px] font-medium text-muted-foreground">{t(key)}</span>;
}

/** Renders the structured ACP output variants supported by this vertical slice. */
function ToolContent({ content }: { content: acp.ToolCallContent }) {
  const { t } = useTranslation();
  switch (content.type) {
    case "diff":
      return <DiffView path={content.path} oldText={content.oldText} newText={content.newText} />;
    case "terminal":
      return <p data-selectable className="flex items-center gap-2 rounded-r-sm border-l-2 border-amber-500/50 bg-muted/25 px-3 py-2 font-mono text-[11px] text-muted-foreground"><IconTerminal2 className="size-3.5" />{t("chat.terminalSession", { id: content.terminalId })}</p>;
    case "content":
      if (content.content.type === "text") {
        return <pre data-selectable className="max-h-72 overflow-auto rounded-r-sm border-l-2 border-border bg-[var(--code-background)] px-3 py-2.5 text-[11px] leading-5 whitespace-pre-wrap">{content.content.text}</pre>;
      }
      return <p className="border-l-2 border-border bg-muted/25 px-3 py-2 text-xs text-muted-foreground">{t("chat.unsupportedContent", { type: content.content.type })}</p>;
  }
}

/** Keeps protocol debugging data available without competing with structured output. */
function RawData({ input, output }: { input: unknown; output: unknown }) {
  const { t } = useTranslation();
  return (
    <details className="border-l-2 border-border bg-muted/20">
      <summary className="cursor-pointer px-3 py-2 text-[11px] font-medium text-muted-foreground">{t("chat.rawData")}</summary>
      <pre data-selectable className="max-h-72 overflow-auto px-3 pb-3 text-[11px] leading-5">{safeStringify({ input, output })}</pre>
    </details>
  );
}

/** Stringifies protocol values while retaining bigint usage fields. */
function safeStringify(value: unknown): string {
  return JSON.stringify(value, (_key, nested) => typeof nested === "bigint" ? nested.toString() : nested, 2);
}

/** Selects a recognizable icon for common ACP tool categories. */
function ToolKindIcon({ kind }: { kind: acp.ToolKind | undefined }) {
  switch (kind) {
    case "read":
      return <IconFileText className="size-4 shrink-0 text-sky-600" />;
    case "edit":
      return <IconFileDiff className="size-4 shrink-0 text-violet-600" />;
    case "delete":
      return <IconTrash className="size-4 shrink-0 text-destructive" />;
    case "move":
      return <IconArrowsMove className="size-4 shrink-0 text-violet-600" />;
    case "search":
      return <IconSearch className="size-4 shrink-0 text-sky-600" />;
    case "execute":
      return <IconTerminal2 className="size-4 shrink-0 text-amber-600" />;
    case "think":
      return <IconCode className="size-4 shrink-0 text-violet-600" />;
    case "fetch":
      return <IconWorld className="size-4 shrink-0 text-sky-600" />;
    case "switch_mode":
      return <IconArrowsExchange className="size-4 shrink-0 text-muted-foreground" />;
    case "other":
    case undefined:
      return <IconTool className="size-4 shrink-0 text-muted-foreground" />;
  }
}

/** Displays tool state with both iconography and localized text. */
export function ToolStatus({ status, compact = false }: { status: acp.ToolCallStatus | undefined; compact?: boolean }) {
  const { t } = useTranslation();
  switch (status) {
    case "completed":
      return <span className="inline-flex shrink-0 items-center gap-1 text-[11px] text-emerald-600"><IconCheck className="size-3" />{compact ? <span className="sr-only">{t("chat.toolCompleted")}</span> : t("chat.toolCompleted")}</span>;
    case "failed":
      return <span className="inline-flex shrink-0 items-center gap-1 text-[11px] text-destructive"><IconAlertTriangle className="size-3" />{compact ? <span className="sr-only">{t("chat.toolFailed")}</span> : t("chat.toolFailed")}</span>;
    case "pending":
      return <span className="shrink-0 text-[11px] text-muted-foreground">{compact ? <span className="sr-only">{t("chat.toolPending")}</span> : t("chat.toolPending")}</span>;
    case "in_progress":
      return <span className="inline-flex shrink-0 items-center gap-1 text-[11px] text-sky-600"><IconLoader2 className="size-3 animate-spin motion-reduce:animate-none" />{compact ? <span className="sr-only">{t("chat.toolRunning")}</span> : t("chat.toolRunning")}</span>;
    case undefined:
      return null;
  }
}
