import { useEffect, useState } from "react";
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
  const [open, setOpen] = useState(!hasFollowingActivity);

  useEffect(() => {
    if (hasFollowingActivity) setOpen(false);
  }, [hasFollowingActivity]);

  return (
    <Collapsible open={open} onOpenChange={setOpen} className="rounded-lg border border-border/70 bg-muted/30">
      <CollapsibleTrigger className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs text-muted-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring">
        <IconBrain className="size-3.5" />
        <span className="font-medium">{hasFollowingActivity ? t("chat.thought") : t("chat.thinking")}</span>
        <IconChevronDown className={`ml-auto size-3.5 transition-transform ${open ? "rotate-180" : ""}`} />
      </CollapsibleTrigger>
      <CollapsibleContent>
        <p className="border-t border-border/60 px-3 py-2.5 text-xs leading-5 text-muted-foreground whitespace-pre-wrap">
          {thought.content}
        </p>
      </CollapsibleContent>
    </Collapsible>
  );
}
