import { useState } from "react";
import { IconListCheck, IconMessage } from "@tabler/icons-react";
import type { acp } from "@ora/contracts";
import { Tooltip, TooltipContent, TooltipTrigger } from "@ora/ui";
import { useTranslation } from "react-i18next";

interface ModeSelectorProps {
  modes: acp.SessionModeState;
  disabled?: boolean;
  onChange: (modeId: acp.SessionModeId) => Promise<void>;
}

/** Presents provider modes as a compact, keyboard-accessible segmented control. */
export function ModeSelector({ modes, disabled = false, onChange }: ModeSelectorProps) {
  const { t } = useTranslation();
  const [pendingModeId, setPendingModeId] = useState<acp.SessionModeId | null>(null);

  const selectMode = async (modeId: acp.SessionModeId) => {
    if (modeId === modes.currentModeId || pendingModeId !== null) return;
    setPendingModeId(modeId);
    try {
      await onChange(modeId);
    } finally {
      setPendingModeId(null);
    }
  };

  if (modes.availableModes.length < 2) return null;

  return (
    <div
      role="radiogroup"
      aria-label={t("chat.mode")}
      className="flex h-8 shrink-0 items-center rounded-md border border-border bg-muted/30 p-0.5"
    >
      {modes.availableModes.map((mode) => {
        const selected = mode.id === modes.currentModeId;
        const isPlan = /plan/i.test(`${mode.id} ${mode.name}`);
        const Icon = isPlan ? IconListCheck : IconMessage;
        return (
          <Tooltip key={mode.id} disabled={!mode.description}>
            <TooltipTrigger
              render={(
                <button
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  disabled={disabled || pendingModeId !== null}
                  onClick={() => void selectMode(mode.id).catch(() => undefined)}
                  className="flex h-7 min-w-16 cursor-pointer items-center justify-center gap-1.5 rounded px-2 text-[11px] font-medium text-muted-foreground outline-none transition-colors duration-150 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default disabled:opacity-50 aria-checked:bg-background aria-checked:text-foreground aria-checked:shadow-sm"
                />
              )}
            >
              <Icon className={`size-3.5 ${pendingModeId === mode.id ? "animate-pulse" : ""}`} aria-hidden="true" />
              <span>{mode.name}</span>
            </TooltipTrigger>
            <TooltipContent sideOffset={8}>{mode.description}</TooltipContent>
          </Tooltip>
        );
      })}
    </div>
  );
}
