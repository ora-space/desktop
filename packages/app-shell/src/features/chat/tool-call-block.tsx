import { useEffect, useState } from "react";
import {
  IconAlertTriangle,
  IconCheck,
  IconChevronDown,
  IconCode,
  IconFile,
  IconLoader2,
  IconSearch,
  IconTerminal2,
  IconTool,
} from "@tabler/icons-react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@ora/ui";
import { useTranslation } from "react-i18next";
import type { ChatToolCall } from "@ora/chat";
import type { acp } from "@ora/contracts";
import { DiffView } from "./diff-view";

interface ToolCallBlockProps {
  tool: ChatToolCall;
}

/** Displays one tool lifecycle and keeps failed or active work visible by default. */
export function ToolCallBlock({ tool }: ToolCallBlockProps) {
  const [open, setOpen] = useState(tool.status !== "completed");

  useEffect(() => {
    if (tool.status === "completed") setOpen(false);
    if (tool.status === "failed") setOpen(true);
  }, [tool.status]);

  return (
    <Collapsible open={open} onOpenChange={setOpen} className="rounded-lg border border-border/80 bg-card">
      <CollapsibleTrigger className="flex w-full items-center gap-2 px-3 py-2.5 text-left text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring">
        <ToolKindIcon kind={tool.toolKind} />
        <span className="min-w-0 flex-1 truncate font-medium">{tool.title}</span>
        <ToolStatus status={tool.status} />
        <IconChevronDown className={`size-3.5 text-muted-foreground transition-transform ${open ? "rotate-180" : ""}`} />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <div className="space-y-3 border-t border-border/70 px-3 py-3">
          {tool.locations.length > 0 && (
            <div className="space-y-1">
              {tool.locations.map((location) => (
                <button key={`${location.path}:${location.line ?? ""}`} type="button" className="block max-w-full truncate font-mono text-[11px] text-sky-700 hover:underline dark:text-sky-400" title={location.path}>
                  {location.path}{location.line === undefined || location.line === null ? "" : `:${location.line}`}
                </button>
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
      return <p className="rounded-md bg-muted px-3 py-2 text-xs text-muted-foreground">{t("chat.unsupportedContent", { type: "terminal" })}</p>;
    case "content":
      if (content.content.type === "text") {
        return <pre className="overflow-x-auto rounded-md bg-muted/60 p-3 text-[11px] leading-5 whitespace-pre-wrap">{content.content.text}</pre>;
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
      <pre className="overflow-x-auto border-t border-border/60 p-3 text-[11px] leading-5">{safeStringify({ input, output })}</pre>
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
    case "edit":
    case "delete":
    case "move":
      return <IconFile className="size-4 text-sky-600" />;
    case "search":
      return <IconSearch className="size-4 text-violet-600" />;
    case "execute":
      return <IconTerminal2 className="size-4 text-amber-600" />;
    case "think":
      return <IconCode className="size-4 text-violet-600" />;
    case "fetch":
    case "switch_mode":
    case "other":
    case undefined:
      return <IconTool className="size-4 text-muted-foreground" />;
  }
}

/** Displays tool state with both iconography and localized text. */
function ToolStatus({ status }: { status: acp.ToolCallStatus | undefined }) {
  const { t } = useTranslation();
  switch (status) {
    case "completed":
      return <span className="inline-flex items-center gap-1 text-[11px] text-emerald-600"><IconCheck className="size-3" />{t("chat.toolCompleted")}</span>;
    case "failed":
      return <span className="inline-flex items-center gap-1 text-[11px] text-destructive"><IconAlertTriangle className="size-3" />{t("chat.toolFailed")}</span>;
    case "pending":
      return <span className="text-[11px] text-muted-foreground">{t("chat.toolPending")}</span>;
    case "in_progress":
      return <span className="inline-flex items-center gap-1 text-[11px] text-sky-600"><IconLoader2 className="size-3 animate-spin" />{t("chat.toolRunning")}</span>;
    case undefined:
      return null;
  }
}
