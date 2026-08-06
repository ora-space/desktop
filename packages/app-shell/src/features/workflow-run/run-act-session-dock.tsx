import { Button, cn } from "@ora/ui";
import {
  IconArrowBackUp,
  IconMessageCircle,
  IconSparkles,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";

export type ActSessionDockTone = "stage" | "hitl";

interface RunActSessionDockProps {
  open: boolean;
  messageCount: number;
  onOpenChange: (open: boolean) => void;
  /** Amber chrome when the dock lives inside HITL action cluster. */
  tone?: ActSessionDockTone;
}

/**
 * Session-mode entry shared by embedded HITL (accessory) and under-stage
 * parallel HITL so gate + conversation stay one action cluster.
 */
export function RunActSessionDock({
  open,
  messageCount,
  onOpenChange,
  tone = "stage",
}: RunActSessionDockProps) {
  const { t } = useTranslation();
  const hitlTone = tone === "hitl";
  const showDockSpark = messageCount === 0;

  return (
    <Button
      type="button"
      variant="ghost"
      size="icon-sm"
      data-hitl-accessory=""
      className={cn(
        "group/session-dock relative shrink-0 cursor-pointer rounded-full",
        "transition-[transform,background-color,border-color,box-shadow,color] duration-200 motion-reduce:transition-none",
        hitlTone
          ? cn(
            "size-8 border border-amber-500/25 bg-background/70 text-amber-950/80",
            "hover:border-amber-500/40 hover:bg-amber-500/10 hover:text-amber-950",
            "dark:text-amber-100/85 dark:hover:text-amber-50",
            open && "border-amber-500/45 bg-amber-500/15 text-amber-950 dark:text-amber-50",
          )
          : cn(
            "ml-auto size-9 border shadow-sm",
            open
              ? "border-primary/20 bg-primary/10 text-primary hover:bg-primary/15"
              : "border-border/80 bg-background hover:-translate-y-0.5 hover:border-primary/30 hover:shadow-md",
          ),
      )}
      aria-expanded={open}
      aria-label={open
        ? t("workflowRun.conversation.backToAct")
        : t("workflowRun.conversation.open")}
      title={open
        ? t("workflowRun.conversation.backToAct")
        : t("workflowRun.conversation.open")}
      onClick={(event) => {
        event.stopPropagation();
        onOpenChange(!open);
      }}
      onKeyDown={(event) => event.stopPropagation()}
      onPointerDown={(event) => event.stopPropagation()}
    >
      {open
        ? <IconArrowBackUp className="size-3.5" />
        : (
          <span className="relative flex size-5 items-center justify-center">
            <IconMessageCircle className={hitlTone ? "size-4" : "size-[18px]"} />
            {showDockSpark && (
              <span
                className={cn(
                  "absolute rounded-full border bg-background p-0.5 shadow-sm",
                  hitlTone
                    ? "-right-1.5 -top-1.5 border-amber-500/25"
                    : "-right-2 -top-2 border-border/60",
                )}
              >
                <IconSparkles
                  className={cn(
                    "size-2.5 transition-transform duration-200 group-hover/session-dock:rotate-12 motion-reduce:transition-none",
                    hitlTone ? "text-amber-700 dark:text-amber-300" : "text-primary/85",
                  )}
                />
              </span>
            )}
            {messageCount > 0 && (
              <span
                className={cn(
                  "absolute flex min-w-4 items-center justify-center rounded-full px-1 text-[9px] font-semibold leading-4",
                  hitlTone
                    ? "-right-1 -top-1 border border-amber-600/20 bg-amber-700 text-amber-50 dark:bg-amber-500 dark:text-amber-950"
                    : "-right-1.5 -top-1 border border-background bg-primary text-primary-foreground",
                )}
              >
                {messageCount}
              </span>
            )}
          </span>
        )}
    </Button>
  );
}
