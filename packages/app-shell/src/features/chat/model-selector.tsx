import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import type { AgentCli } from "@ora/contracts";
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
import { useChatStore } from "../../chat-store-context";
import { useSettingsStore } from "../../state/stores/settings-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useSetSessionConfig } from "../../state/hooks/use-session-config";
import { useWarmSession } from "../../state/hooks/use-warm-session";
import { useSwitchSessionAgent } from "../../state/hooks/use-workspace-mutations";
import {
  AGENT_CLI_LABELS,
  AGENT_CLI_ORDER,
  currentValueName,
  findModelOption,
  selectableValues,
} from "./model-catalog";
import { ProviderLogo } from "./provider-logos";

/**
 * The composer's agent and model picker.
 *
 * Both lists describe the session the composer will send into. Which CLIs exist
 * is static; which models are available is whatever that CLI reported for this
 * session, so the model list is empty until a session has been established.
 *
 * With a session selected, choosing a different CLI moves that conversation onto
 * it rather than only changing the default for the next one. Ora owns the
 * transcript, so the thread survives the move: the backend hands it to the new
 * agent with the user's next message.
 */
export function ModelSelector({ disabled = false }: { disabled?: boolean }) {
  const { t } = useTranslation();
  const agentCli = useSettingsStore((state) => state.settings.agentCli);
  const updateSettings = useSettingsStore((state) => state.updateSettings);
  const selection = useWorkspaceSelectionStore((state) => state.selection);
  const chatStore = useChatStore();
  const setSessionConfig = useSetSessionConfig();
  const switchAgent = useSwitchSessionAgent();

  // Shares the workspace's warm-session query key, so this is a cache read
  // rather than a second provider session.
  const warmSessionId = useWarmSession(selection, agentCli);
  const activeSessionId = selection.sessionId ?? warmSessionId;
  const configOptions = useStore(chatStore, (state) =>
    activeSessionId === null ? undefined : state.conversations[activeSessionId]?.configOptions,
  );
  const modelOption = configOptions ? findModelOption(configOptions) : null;

  const activeLabel = modelOption
    ? currentValueName(modelOption)
    : t("chat.modelSelector.placeholder");

  /**
   * A persisted session is moved onto the chosen CLI rather than left behind;
   * a warm one only needs the new default, which re-warms it against that CLI.
   */
  const selectAgent = (candidate: AgentCli) => {
    updateSettings({ agentCli: candidate });
    if (selection.sessionId !== null && candidate !== agentCli) {
      switchAgent.mutate({ sessionId: selection.sessionId, agentCli: candidate });
    }
  };

  const selectModel = (value: string) => {
    if (activeSessionId === null || modelOption === null) return;
    setSessionConfig.mutate({
      sessionId: activeSessionId,
      configId: modelOption.id,
      value,
    });
  };

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
        {setSessionConfig.isPending || switchAgent.isPending
          ? <IconLoader2 className="size-3 shrink-0 animate-spin opacity-50" aria-hidden="true" />
          : <IconChevronDown className="size-3 shrink-0 opacity-50" aria-hidden="true" />}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" side="top" className="w-56">
        <DropdownMenuGroup className="p-1">
          <DropdownMenuLabel className="px-2 py-1.5 text-xs font-normal text-muted-foreground">
            {t("chat.modelSelector.agent")}
          </DropdownMenuLabel>
          {AGENT_CLI_ORDER.map((candidate) => (
            <DropdownMenuItem
              key={candidate}
              className="gap-1.5 rounded-sm px-2 py-1.5 text-xs"
              onClick={() => selectAgent(candidate)}
            >
              <ProviderLogo agentCli={candidate} className="size-3.5" />
              {AGENT_CLI_LABELS[candidate]}
              {candidate === agentCli && <IconCheck className="ml-auto size-4" />}
            </DropdownMenuItem>
          ))}
        </DropdownMenuGroup>
        <DropdownMenuGroup className="p-1">
          <DropdownMenuLabel className="px-2 py-1.5 text-xs font-normal text-muted-foreground">
            {t("chat.modelSelector.model")}
          </DropdownMenuLabel>
          {modelOption === null ? (
            <p className="px-2 py-4 text-center text-xs text-muted-foreground">
              {t("chat.modelSelector.empty")}
            </p>
          ) : (
            selectableValues(modelOption).map((value) => (
              <DropdownMenuItem
                key={value.value}
                className="gap-1.5 rounded-sm px-2 py-1.5 text-xs"
                onClick={() => selectModel(value.value)}
              >
                {value.name}
                {modelOption.type === "select" && value.value === modelOption.currentValue && (
                  <IconCheck className="ml-auto size-4" />
                )}
              </DropdownMenuItem>
            ))
          )}
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
