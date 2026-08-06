import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Badge, Button, cn } from "@ora/ui";
import { IconChevronDown, IconChevronRight } from "@tabler/icons-react";
import { formatRunClock } from "../../lib/format";
import type { WorkflowArtifact } from "@ora/workflow-runtime";

interface RunActArtifactsProps {
  artifacts: WorkflowArtifact[];
  /** Artifact id that just arrived —opens and animates that item. */
  revealedId: string | null;
  /**
   * When true, omit the outer section chrome (parent already provides a heading).
   */
  embedded?: boolean;
}

/**
 * Progressive disclosure list for node-scoped outcomes.
 * Used inside the Theater act inspector.
 */
export function RunActArtifacts({
  artifacts,
  revealedId,
  embedded = false,
}: RunActArtifactsProps) {
  const { t } = useTranslation();

  if (artifacts.length === 0) {
    return null;
  }

  const list = (
    <ul className={cn("space-y-2", !embedded && "mt-3")}>
      {artifacts.map((artifact) => (
        <li key={artifact.id}>
          <ActArtifactItem
            artifact={artifact}
            reveal={artifact.id === revealedId}
          />
        </li>
      ))}
    </ul>
  );

  if (embedded) {
    return list;
  }

  return (
    <section
      className="rounded-xl border border-border/70 bg-background/70 px-4 py-3"
      aria-label={t("workflowRun.artifacts.title")}
    >
      <div className="flex items-center gap-2">
        <p className="text-[11px] font-medium uppercase tracking-[0.04em] text-muted-foreground">
          {t("workflowRun.artifacts.title")}
        </p>
        <span className="tabular-nums text-[10px] text-muted-foreground/80">
          {artifacts.length}
        </span>
      </div>
      {list}
    </section>
  );
}

function ActArtifactItem({
  artifact,
  reveal,
}: {
  artifact: WorkflowArtifact;
  reveal: boolean;
}) {
  const { i18n, t } = useTranslation();
  const locale = i18n.resolvedLanguage === "en-US" ? "en-US" : undefined;
  const [open, setOpen] = useState(reveal);
  const [animate, setAnimate] = useState(reveal);
  // React's documented "adjusting state when a prop changes" pattern: hoist the
  // reveal transition out of an effect so it does not cascade a render.
  const [previousReveal, setPreviousReveal] = useState(reveal);
  if (reveal !== previousReveal) {
    setPreviousReveal(reveal);
    if (reveal) {
      setOpen(true);
      setAnimate(true);
    }
  }

  useEffect(() => {
    if (!animate) {
      return;
    }
    const timer = window.setTimeout(() => setAnimate(false), 420);
    return () => window.clearTimeout(timer);
  }, [animate]);

  return (
    <div
      className={cn(
        "overflow-hidden rounded-lg border border-border/50 bg-transparent",
        open && "bg-muted/20",
        animate
          && "animate-in fade-in slide-in-from-bottom-1 duration-300 ease-out motion-reduce:animate-none",
      )}
      data-reveal={animate ? "" : undefined}
    >
      <Button
        type="button"
        variant="ghost"
        className="h-auto w-full cursor-pointer justify-start gap-2 rounded-none px-3 py-2 text-left hover:bg-muted/50"
        aria-expanded={open}
        onClick={() => setOpen((value) => !value)}
      >
        {open
          ? <IconChevronDown className="size-3.5 shrink-0 text-muted-foreground" />
          : <IconChevronRight className="size-3.5 shrink-0 text-muted-foreground" />}
        <span className="min-w-0 flex-1 truncate text-xs font-medium">
          {artifact.title}
        </span>
        <Badge variant="secondary" className="shrink-0 text-[9px]">
          {t(`workflowRun.artifacts.kind.${artifact.kind}`)}
        </Badge>
        <span className="shrink-0 text-[10px] tabular-nums text-muted-foreground">
          {formatRunClock(artifact.createdAt, locale)}
        </span>
      </Button>
      {open && (
        <pre
          data-selectable
          className="max-h-48 overflow-auto border-t border-border/50 px-3 py-2.5 font-sans text-[11px] leading-5 whitespace-pre-wrap text-foreground/90"
        >
          {artifact.body}
        </pre>
      )}
    </div>
  );
}
