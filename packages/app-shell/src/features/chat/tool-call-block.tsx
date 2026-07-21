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
  appearance?: "standalone" | "embedded";
}

/** Displays one tool lifecycle and keeps failed or active work visible by default. */
export function ToolCallBlock({ tool, appearance = "standalone" }: ToolCallBlockProps) {
  const [disclosure, setDisclosure] = useState({ status: tool.status, open: tool.status !== "completed" });
  if (disclosure.status !== tool.status) {
    setDisclosure({
      status: tool.status,
      open: tool.status !== "completed",
    });
  }
  const open = disclosure.open;
  const hasDetails = tool.locations.length > 0 || tool.content.length > 0 || tool.rawInput !== undefined || tool.rawOutput !== undefined;
  const summary = (
    <>
      <ToolKindIcon kind={tool.toolKind} />
      <span className="min-w-0 flex-1 truncate font-medium" title={tool.title}>{tool.title}</span>
      <ToolStatus status={tool.status} />
      {hasDetails && <IconChevronDown className={`size-3.5 shrink-0 text-muted-foreground transition-transform motion-reduce:transition-none ${open ? "rotate-180" : ""}`} />}
    </>
  );

  if (!hasDetails) {
    return (
      <div className={`flex min-h-10 w-full items-center gap-2 px-3 py-2 text-xs ${appearance === "standalone" ? "rounded-md border border-border/70 bg-card" : "bg-transparent"}`}>
        {summary}
      </div>
    );
  }

  return (
    <Collapsible
      open={open}
      onOpenChange={(nextOpen) => setDisclosure({ status: tool.status, open: nextOpen })}
      className={appearance === "standalone" ? "rounded-md border border-border/80 bg-card" : "bg-transparent"}
    >
      <CollapsibleTrigger className="flex min-h-11 w-full items-center gap-2 px-3 py-2 text-left text-xs outline-none transition-colors hover:bg-muted/30 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring">
        {summary}
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className={`space-y-3 border-t border-border/70 px-3 py-3 ${appearance === "embedded" ? "bg-background/45 pl-9" : ""}`}>
          {tool.locations.length > 0 && (
            <div className="space-y-1">
              {tool.locations.map((location) => (
                <code key={`${location.path}:${location.line ?? ""}`} className="block max-w-full truncate text-[11px] text-sky-700 dark:text-sky-400" title={location.path}>
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

/** Renders the structured ACP output variants supported by this vertical slice. */
function ToolContent({ content }: { content: acp.ToolCallContent }) {
  const { t } = useTranslation();
  switch (content.type) {
    case "diff":
      return <DiffView path={content.path} oldText={content.oldText} newText={content.newText} />;
    case "terminal":
      return <p className="flex items-center gap-2 rounded-md border border-border/70 bg-muted/40 px-3 py-2 font-mono text-[11px] text-muted-foreground"><IconTerminal2 className="size-3.5" />{t("chat.terminalSession", { id: content.terminalId })}</p>;
    case "content":
      if (content.content.type === "text") {
        return <pre className="max-h-72 overflow-auto rounded-md border border-border/60 bg-muted/45 p-3 text-[11px] leading-5 whitespace-pre-wrap">{content.content.text}</pre>;
      }
      return <p className="rounded-md bg-muted px-3 py-2 text-xs text-muted-foreground">{t("chat.unsupportedContent", { type: content.content.type })}</p>;
  }
}

/** Keeps protocol debugging data available without competing with structured output. */
function RawData({ input, output }: { input: unknown; output: unknown }) {
  const { t } = useTranslation();
  return (
    <details className="rounded-md border border-border/70 bg-muted/20">
      <summary className="cursor-pointer px-3 py-2 text-[11px] font-medium text-muted-foreground">{t("chat.rawData")}</summary>
      <pre className="max-h-72 overflow-auto border-t border-border/60 p-3 text-[11px] leading-5">{safeStringify({ input, output })}</pre>
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
export function ToolStatus({ status }: { status: acp.ToolCallStatus | undefined }) {
  const { t } = useTranslation();
  switch (status) {
    case "completed":
      return <span className="inline-flex shrink-0 items-center gap-1 text-[11px] text-emerald-600"><IconCheck className="size-3" />{t("chat.toolCompleted")}</span>;
    case "failed":
      return <span className="inline-flex shrink-0 items-center gap-1 text-[11px] text-destructive"><IconAlertTriangle className="size-3" />{t("chat.toolFailed")}</span>;
    case "pending":
      return <span className="shrink-0 text-[11px] text-muted-foreground">{t("chat.toolPending")}</span>;
    case "in_progress":
      return <span className="inline-flex shrink-0 items-center gap-1 text-[11px] text-sky-600"><IconLoader2 className="size-3 animate-spin motion-reduce:animate-none" />{t("chat.toolRunning")}</span>;
    case undefined:
      return null;
  }
}
