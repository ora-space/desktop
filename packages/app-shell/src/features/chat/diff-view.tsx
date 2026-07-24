import { useMemo, useState } from "react";
import { diffLines } from "diff";
import { Button } from "@ora/ui";
import { useTranslation } from "react-i18next";

interface DiffViewProps {
  path: string;
  oldText: string | null | undefined;
  newText: string;
}

interface DisplayLine {
  kind: "context" | "added" | "removed" | "separator";
  oldLine: number | null;
  newLine: number | null;
  text: string;
}

const DIFF_CONTEXT_LINES = 3;

/** Renders a compact unified line diff derived by the maintained `diff` package. */
export function DiffView({ path, oldText, newText }: DiffViewProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const lines = useMemo(() => buildDisplayLines(oldText ?? "", newText), [oldText, newText]);
  const compactLines = useMemo(() => selectChangedHunks(lines), [lines]);
  const additions = lines.filter((line) => line.kind === "added").length;
  const deletions = lines.filter((line) => line.kind === "removed").length;
  const visibleLines = expanded ? lines : compactLines;

  return (
    <div className="overflow-hidden rounded-md border border-border bg-background">
      <div className="flex items-center gap-2 border-b border-border bg-muted/50 px-3 py-2 text-[11px]">
        <span data-selectable className="min-w-0 flex-1 truncate font-mono" title={path}>{path}</span>
        <span className="text-emerald-600">+{additions}</span>
        <span className="text-red-600">-{deletions}</span>
      </div>
      <div className="overflow-x-auto font-mono text-[11px] leading-5">
        {visibleLines.map((line, index) => (
          <div
            key={`${index}-${line.kind}-${line.oldLine}-${line.newLine}`}
            className={
              line.kind === "added"
                ? "flex min-w-max bg-emerald-500/10 text-emerald-950 dark:text-emerald-100"
                : line.kind === "removed"
                  ? "flex min-w-max bg-red-500/10 text-red-950 dark:text-red-100"
                  : line.kind === "separator"
                    ? "flex min-w-max bg-muted/40 text-muted-foreground"
                  : "flex min-w-max text-muted-foreground"
            }
          >
            <span className="w-10 shrink-0 select-none border-r border-border/50 px-2 text-right text-muted-foreground/70">{line.oldLine ?? ""}</span>
            <span className="w-10 shrink-0 select-none border-r border-border/50 px-2 text-right text-muted-foreground/70">{line.newLine ?? ""}</span>
            <span className="w-5 shrink-0 select-none text-center">{line.kind === "added" ? "+" : line.kind === "removed" ? "-" : " "}</span>
            <span data-selectable className="pr-4 whitespace-pre">{line.text}</span>
          </div>
        ))}
      </div>
      {compactLines.length < lines.length && (
        <div className="border-t border-border px-2 py-1.5">
          <Button variant="ghost" size="sm" onClick={() => setExpanded((current) => !current)}>
            {expanded ? t("chat.diffCollapse") : t("chat.diffExpand")}
          </Button>
        </div>
      )}
    </div>
  );
}

/** Keeps changed lines and nearby context, inserting separators between distant hunks. */
function selectChangedHunks(lines: DisplayLine[]): DisplayLine[] {
  const visibleIndexes = new Set<number>();
  lines.forEach((line, index) => {
    if (line.kind === "context") return;
    const start = Math.max(0, index - DIFF_CONTEXT_LINES);
    const end = Math.min(lines.length - 1, index + DIFF_CONTEXT_LINES);
    for (let visibleIndex = start; visibleIndex <= end; visibleIndex += 1) {
      visibleIndexes.add(visibleIndex);
    }
  });
  if (visibleIndexes.size === 0 || visibleIndexes.size === lines.length) return lines;

  const compact: DisplayLine[] = [];
  let previousIndex = -1;
  for (const index of [...visibleIndexes].sort((left, right) => left - right)) {
    if (previousIndex !== -1 && index > previousIndex + 1) {
      compact.push({ kind: "separator", oldLine: null, newLine: null, text: "…" });
    }
    compact.push(lines[index]!);
    previousIndex = index;
  }
  return compact;
}

/** Converts line-level diff parts into unified rows with stable old/new line numbers. */
function buildDisplayLines(oldText: string, newText: string): DisplayLine[] {
  const lines: DisplayLine[] = [];
  let oldLine = 1;
  let newLine = 1;
  for (const part of diffLines(oldText, newText)) {
    const values = part.value.endsWith("\n")
      ? part.value.slice(0, -1).split("\n")
      : part.value.split("\n");
    for (const text of values) {
      if (part.added) {
        lines.push({ kind: "added", oldLine: null, newLine, text });
        newLine += 1;
      } else if (part.removed) {
        lines.push({ kind: "removed", oldLine, newLine: null, text });
        oldLine += 1;
      } else {
        lines.push({ kind: "context", oldLine, newLine, text });
        oldLine += 1;
        newLine += 1;
      }
    }
  }
  return lines;
}
