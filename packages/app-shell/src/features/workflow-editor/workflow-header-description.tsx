import { useTranslation } from "react-i18next";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@ora/ui";

/** Pause before the full description appears so the header stays quiet on pass-through. */
const DESCRIPTION_TOOLTIP_DELAY_MS = 400;

/**
 * Truncates the editor header description and reveals the full text on hover.
 *
 * The identity column shares the title strip with save/export actions, so long
 * copy must ellipsize in place rather than pushing those controls off-screen.
 */
export function WorkflowHeaderDescription({ text }: { text: string }) {
  const { t } = useTranslation();
  return (
    <TooltipProvider delay={DESCRIPTION_TOOLTIP_DELAY_MS}>
      <Tooltip>
        <TooltipTrigger
          render={
            <p
              className="truncate px-1 text-[11px] leading-4 text-muted-foreground"
              aria-label={t("settings.workflow.headerDescription")}
            >
              {text}
            </p>
          }
        />
        <TooltipContent
          side="bottom"
          align="start"
          sideOffset={6}
          className="max-h-48 max-w-sm overflow-y-auto whitespace-normal break-words text-left"
        >
          {text}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
