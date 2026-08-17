import { useMemo, useState } from "react";
import {
  IconChevronDown,
  IconChevronRight,
  IconFileDiff,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import type { ChatTurn } from "@ora/chat";
import { useTaskChangesNavigation } from "../diff/task-changes-navigation-context";
import { collectTurnDiffFiles } from "./turn-diff-files";

interface TurnDiffSummaryProps {
  turn: ChatTurn;
}

/** Shows the completed file changes owned by one response and opens them in task Changes. */
export function TurnDiffSummary({ turn }: TurnDiffSummaryProps) {
  const { t } = useTranslation();
  const changesNavigation = useTaskChangesNavigation();
  const files = useMemo(() => collectTurnDiffFiles(turn), [turn]);
  const [expanded, setExpanded] = useState(true);
  const totals = files.reduce(
    (result, file) => ({
      additions: result.additions + file.additions,
      deletions: result.deletions + file.deletions,
    }),
    { additions: 0, deletions: 0 },
  );

  if (turn.status === "streaming" || files.length === 0) return null;

  return (
    <section
      aria-label={t("chat.turnDiff.title", { count: files.length })}
      className="overflow-hidden rounded-lg border border-border/80 bg-background shadow-xs"
    >
      <header className="border-b border-border/70 bg-muted/20">
        <button
          type="button"
          className="flex min-h-11 w-full items-center gap-2.5 px-3 py-2 text-left outline-none transition-colors hover:bg-muted/35 focus-visible:bg-muted/35 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
          aria-expanded={expanded}
          aria-label={t(
            expanded ? "chat.turnDiff.collapse" : "chat.turnDiff.expand",
          )}
          onClick={() => setExpanded((current) => !current)}
        >
          <IconChevronDown
            className={`size-3.5 shrink-0 text-muted-foreground transition-transform ${expanded ? "" : "-rotate-90"}`}
            aria-hidden="true"
          />
          <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-violet-500/10 text-violet-700 dark:text-violet-400">
            <IconFileDiff className="size-4" />
          </span>
          <span className="min-w-0 flex-1 text-xs font-medium">
            {t("chat.turnDiff.title", { count: files.length })}
          </span>
          <span
            className="flex shrink-0 gap-1.5 font-mono text-[10px]"
            aria-label={t("chat.toolGroup.changeStats", totals)}
          >
            <span className="text-emerald-600">+{totals.additions}</span>
            <span className="text-red-600">-{totals.deletions}</span>
          </span>
        </button>
      </header>
      {expanded && (
        <div className="divide-y divide-border/60">
          {files.map((file) => (
            <button
              key={file.path}
              type="button"
              className="flex min-h-9 w-full items-center gap-3 px-3 py-2 text-left outline-none transition-colors hover:bg-muted/35 focus-visible:bg-muted/35 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
              onClick={() => changesNavigation?.openDiff(file.path)}
            >
              <span
                className="min-w-0 flex-1 truncate font-mono text-[11px]"
                title={file.path}
              >
                {file.path}
              </span>
              <span
                className="flex shrink-0 gap-1.5 font-mono text-[10px]"
                aria-label={t("chat.toolGroup.changeStats", {
                  additions: file.additions,
                  deletions: file.deletions,
                })}
              >
                <span className="text-emerald-600">+{file.additions}</span>
                <span className="text-red-600">-{file.deletions}</span>
              </span>
              <IconChevronRight className="size-3.5 shrink-0 text-muted-foreground" />
            </button>
          ))}
        </div>
      )}
    </section>
  );
}
