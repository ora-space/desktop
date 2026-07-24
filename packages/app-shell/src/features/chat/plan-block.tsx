import { useState } from "react";
import { IconCheck, IconChevronDown, IconCircle, IconLoader2, IconListCheck } from "@tabler/icons-react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@ora/ui";
import { useTranslation } from "react-i18next";
import type { ChatPlan } from "@ora/chat";
import type { acp } from "@ora/contracts";

interface PlanBlockProps {
  plan: ChatPlan;
}

/** Displays the latest complete plan snapshot and collapses once every entry finishes. */
export function PlanBlock({ plan }: PlanBlockProps) {
  const { t } = useTranslation();
  const completed = plan.entries.filter((entry) => entry.status === "completed").length;
  const allCompleted = plan.entries.length > 0 && completed === plan.entries.length;
  const [disclosure, setDisclosure] = useState({ allCompleted, open: !allCompleted });
  if (disclosure.allCompleted !== allCompleted) {
    setDisclosure({ allCompleted, open: !allCompleted });
  }
  const open = disclosure.open;

  return (
    <Collapsible open={open} onOpenChange={(nextOpen) => setDisclosure({ allCompleted, open: nextOpen })} className="rounded-md border border-border/80 bg-card">
      <CollapsibleTrigger className="flex min-h-11 w-full items-center gap-2 px-3 py-2 text-left text-xs outline-none transition-colors hover:bg-muted/30 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring">
        <IconListCheck className="size-4 text-sky-600" />
        <span className="font-medium">{t("chat.plan")}</span>
        <span className="text-muted-foreground">{t("chat.planProgress", { completed, total: plan.entries.length })}</span>
        <IconChevronDown className={`ml-auto size-3.5 text-muted-foreground transition-transform motion-reduce:transition-none ${open ? "rotate-180" : ""}`} />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <ol className="space-y-2 border-t border-border/70 px-3 py-3">
          {plan.entries.map((entry, index) => (
            <li key={`${index}-${entry.content}`} className="flex items-start gap-2 text-xs leading-5">
              <PlanStatusIcon status={entry.status} />
              <span data-selectable className={entry.status === "completed" ? "text-muted-foreground line-through" : "text-foreground"}>
                {entry.content}
              </span>
            </li>
          ))}
        </ol>
      </CollapsibleContent>
    </Collapsible>
  );
}

/** Maps plan status to a compact icon without relying on color alone. */
function PlanStatusIcon({ status }: { status: acp.PlanEntryStatus }) {
  switch (status) {
    case "pending":
      return <IconCircle className="mt-1 size-3 shrink-0 text-muted-foreground" />;
    case "in_progress":
      return <IconLoader2 className="mt-1 size-3 shrink-0 animate-spin text-sky-600 motion-reduce:animate-none" />;
    case "completed":
      return <IconCheck className="mt-1 size-3 shrink-0 text-emerald-600" />;
  }
}
