import { useTranslation } from "react-i18next";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from "@ora/ui";
import { IconCheck, IconChevronDown, IconLoader2 } from "@tabler/icons-react";
import type { AgentCli } from "@ora/contracts";
import { useSettingsStore } from "../../state/stores/settings-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useSwitchSessionAgent } from "../../state/hooks/use-workspace-mutations";
import { AGENT_CLI_LABELS, orderedGroups, useAvailableModels } from "./model-catalog";
import { ProviderLogo } from "./provider-logos";

/**
 * The composer's model picker. It fetches live agent CLI model lists from the
 * backend and groups them by CLI. The active selection is persisted in the
 * settings store so the composer, settings dialog, and session creation all
 * stay in sync.
 *
 * With a session selected, choosing a different CLI moves that conversation onto
 * it rather than only changing the default for the next one. Ora owns the
 * transcript, so the thread survives the move: the backend hands it to the new
 * agent with the user's next message.
 */
export function ModelSelector({ disabled = false }: { disabled?: boolean }) {
  const { t } = useTranslation();
  const agentCli = useSettingsStore((state) => state.settings.agentCli);
  const model = useSettingsStore((state) => state.settings.model);
  const updateSettings = useSettingsStore((state) => state.updateSettings);
  const sessionId = useWorkspaceSelectionStore((state) => state.selection.sessionId);
  const switchAgent = useSwitchSessionAgent();
  const { data: groups, isLoading } = useAvailableModels();

  const selectModel = (nextCli: AgentCli, nextModel: string) => {
    updateSettings({ agentCli: nextCli, model: nextModel });
    if (sessionId !== null && nextCli !== agentCli) {
      switchAgent.mutate({ sessionId, agentCli: nextCli });
    }
  };

  // Pick a representative label for the collapsed trigger.
  const activeLabel = model || (isLoading ? t("chat.modelSelector.loading") : t("chat.modelSelector.placeholder"));

  const visibleGroups = groups && groups.length > 0
    ? orderedGroups(groups, agentCli)
    : [];

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={disabled}
            aria-label={t("chat.modelSelector.label")}
            className="group/model h-7 gap-1.5 rounded-md px-2 text-xs font-normal text-muted-foreground hover:text-foreground"
          />
        }
      >
        {agentCli && <ProviderLogo agentCli={agentCli} className="size-3.5 shrink-0" />}
        {/* The CLI name is width-animated in via a 0fr → 1fr grid so the
            button grows smoothly on hover instead of snapping wider. */}
        <span className="grid grid-cols-[0fr] opacity-0 transition-all duration-200 group-hover/model:grid-cols-[1fr] group-hover/model:opacity-100 group-aria-expanded/model:grid-cols-[1fr] group-aria-expanded/model:opacity-100">
          <span className="min-w-0 overflow-hidden whitespace-nowrap">
            {agentCli ? AGENT_CLI_LABELS[agentCli] : ""}
          </span>
        </span>
        <span className="whitespace-nowrap">{activeLabel}</span>
        {isLoading || switchAgent.isPending
          ? <IconLoader2 className="size-3 shrink-0 animate-spin opacity-50" aria-hidden="true" />
          : <IconChevronDown className="size-3 shrink-0 opacity-50" aria-hidden="true" />}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" side="top" className="w-56">
        {isLoading && (
          <div className="flex items-center justify-center gap-2 px-2 py-6 text-xs text-muted-foreground">
            <IconLoader2 className="size-3.5 animate-spin" />
            {t("chat.modelSelector.loading")}
          </div>
        )}
        {!isLoading && visibleGroups.length === 0 && (
          <p className="px-2 py-4 text-center text-xs text-muted-foreground">
            {t("chat.modelSelector.empty")}
          </p>
        )}
        {visibleGroups.map((group) => (
          <DropdownMenuGroup key={group.agentCli} className="p-1">
            <DropdownMenuLabel className="flex items-center gap-1.5 px-2 py-1.5 text-xs font-normal text-muted-foreground">
              <ProviderLogo agentCli={group.agentCli} className="size-3.5" />
              {AGENT_CLI_LABELS[group.agentCli]}
            </DropdownMenuLabel>
            {group.models.map((candidateModel) => (
              <DropdownMenuItem
                key={`${group.agentCli}:${candidateModel}`}
                className="gap-1.5 rounded-sm px-2 py-1.5 text-xs"
                onClick={() => selectModel(group.agentCli, candidateModel)}
              >
                {candidateModel}
                {group.agentCli === agentCli && candidateModel === model && (
                  <IconCheck className="ml-auto size-4" />
                )}
              </DropdownMenuItem>
            ))}
          </DropdownMenuGroup>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
