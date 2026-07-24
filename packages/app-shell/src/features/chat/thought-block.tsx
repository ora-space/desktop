import { useState } from "react";
import { IconBrain, IconChevronDown } from "@tabler/icons-react";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@ora/ui";
import { useTranslation } from "react-i18next";
import type { ChatThought } from "@ora/chat";

interface ThoughtBlockProps {
  thought: ChatThought;
  hasFollowingActivity: boolean;
}

/** Shows streamed agent analysis as secondary progress that yields to concrete work. */
export function ThoughtBlock({ thought, hasFollowingActivity }: ThoughtBlockProps) {
  const { t } = useTranslation();
  const [disclosure, setDisclosure] = useState({ hasFollowingActivity, open: !hasFollowingActivity });
  if (disclosure.hasFollowingActivity !== hasFollowingActivity) {
    setDisclosure({ hasFollowingActivity, open: !hasFollowingActivity });
  }
  const open = disclosure.open;

  return (
    <Collapsible open={open} onOpenChange={(nextOpen) => setDisclosure({ hasFollowingActivity, open: nextOpen })} className="rounded-md border border-border/70 bg-muted/20">
      <CollapsibleTrigger className="flex min-h-11 w-full items-center gap-2 px-3 py-2 text-left text-xs text-muted-foreground outline-none transition-colors hover:bg-muted/30 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring">
        <IconBrain className="size-3.5" />
        <span className="font-medium">{hasFollowingActivity ? t("chat.thought") : t("chat.thinking")}</span>
        <IconChevronDown className={`ml-auto size-3.5 transition-transform motion-reduce:transition-none ${open ? "rotate-180" : ""}`} />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <p data-selectable className="border-t border-border/60 px-3 py-2.5 text-xs leading-5 text-muted-foreground whitespace-pre-wrap">
          {thought.content}
        </p>
      </CollapsibleContent>
    </Collapsible>
  );
}
