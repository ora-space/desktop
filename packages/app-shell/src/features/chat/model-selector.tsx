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
import { useSessions } from "../../state/hooks/use-sessions";
import { useSetSessionConfig } from "../../state/hooks/use-session-config";
import { useWarmSession, warmTargetKey } from "../../state/hooks/use-warm-session";
import { useTargetAgentCli } from "../../state/hooks/use-target-agent-cli";
import { usePendingAgentStore, usePendingSwitch } from "../../state/stores/pending-agent-store";
import { currentValueName, findModelOption, selectableValues } from "@ora/chat";
import { AGENT_CLI_LABELS, AGENT_CLI_ORDER } from "./model-catalog";
import { ProviderLogo } from "./provider-logos";

/**
 * The composer's agent and model picker.
 *
 * Both lists describe the session the composer will send into. Which CLIs exist
 * is static; which models are available is whatever that CLI reported for this
 * session, so the model list has three states rather than two — still arriving,
 * genuinely offering no choice, or a real set to pick from.
 *
 * With a session selected, choosing a different CLI moves that conversation onto
 * it rather than only changing the default for the next one. Ora owns the
 * transcript, so the thread survives the move: the backend hands it to the new
 * agent with the user's next message. The move is *recorded* here and performed
 * by that message — clicking cannot rebind immediately without tearing down an
 * agent that may still be mid-reply. What the click does start is warming the
 * incoming CLI, which is how the models below can be replaced by its own before
 * anything is committed. Choosing a CLI therefore leaves the menu open: picking
 * one of those models is the other half of the same decision.
 */
export function ModelSelector({ disabled = false }: { disabled?: boolean }) {
  const { t } = useTranslation();
  const updateSettings = useSettingsStore((state) => state.updateSettings);
  const selection = useWorkspaceSelectionStore((state) => state.selection);
  const chatStore = useChatStore();
  const setSessionConfig = useSetSessionConfig();
  const { data: sessions = [] } = useSessions();

  // Having a binding is what makes a session persisted, and only a persisted one
  // can be rebound; a warm session has no row to move.
  const isPersisted = sessions.some((session) => session.id === selection.sessionId);
  const targetKey = warmTargetKey(selection);
  const setPickedForTarget = usePendingAgentStore((state) => state.setPendingAgent);
  const setPendingSwitch = usePendingAgentStore((state) => state.setPendingSwitch);
  const pendingSwitch = usePendingSwitch(selection.sessionId);
  // Resolved centrally so this and the composer cannot disagree: they share one
  // warm-session query key, and the CLI is part of that key.
  const agentCli = useTargetAgentCli(selection);

  // Shares the workspace's warm-session query key, so this is a cache read
  // rather than a second provider session.
  const warmSession = useWarmSession(selection, agentCli);
  // A warm session, when there is one, always describes the CLI on screen —
  // including the one a pending move is heading for, whose models and model
  // choice live on it rather than on the session being moved. While that
  // handshake is still running there is nothing to read: naming the bound
  // session here instead would advertise the outgoing agent's model as the
  // incoming agent's.
  const activeSessionId =
    warmSession.sessionId ?? (pendingSwitch === undefined ? selection.sessionId : null);
  // Selected narrowly rather than as one conversation object, so a streaming
  // turn does not re-render the picker on every token.
  const configOptions = useStore(chatStore, (state) =>
    activeSessionId === null ? undefined : state.conversations[activeSessionId]?.configOptions,
  );
  const isReplayingHistory = useStore(chatStore, (state) =>
    activeSessionId === null
      ? false
      : state.conversations[activeSessionId]?.isLoading === true,
  );
  const modelOption = configOptions ? findModelOption(configOptions) : null;

  // An agent only reports its models as part of the handshake — warming this
  // surface's session, or replaying a selected one — so until that lands the
  // list is unknown rather than empty. Saying "no models" here would answer a
  // question that has not been asked yet, and a handshake can take a second.
  // Replay is its own case: it seeds the conversation with empty options first,
  // which would otherwise read as a settled answer while the stream is still
  // running. A surface that never started warming, or whose handshake failed,
  // is not loading and still reports empty. A pending move reads as loading for
  // the same reason: it has no session to name until the incoming CLI answers.
  const isLoadingModels =
    activeSessionId === null
      ? warmSession.isOpening || pendingSwitch !== undefined
      : configOptions === undefined || isReplayingHistory;

  const activeLabel = modelOption
    ? currentValueName(modelOption)
    : t(isLoadingModels ? "chat.modelSelector.loading" : "chat.modelSelector.placeholder");

  /**
   * A persisted session records a move onto the chosen CLI, to be performed by
   * the next message sent into it; a not-yet-started chat records the pick
   * against its own target instead, so it survives navigating away and back
   * without touching any other chat.
   *
   * Either way the shared default also moves, so the next chat surface no one
   * has touched yet still opens on whatever the user picked most recently, and
   * either way the CLI's own models arrive from a handshake that has not
   * happened yet — this only points the surface at it. The list below settles
   * when that answers, which is why the menu is still open to see it.
   */
  const selectAgent = (candidate: AgentCli) => {
    if (candidate === agentCli) return;
    updateSettings({ agentCli: candidate });
    if (isPersisted && selection.sessionId !== null) {
      setPendingSwitch(selection.sessionId, candidate);
      return;
    }
    if (targetKey !== null) setPickedForTarget(targetKey, candidate);
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
        {setSessionConfig.isPending || isLoadingModels
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
              // Choosing a CLI is only half the choice: its models replace the
              // group below and the user still has to pick one from them.
              closeOnClick={false}
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
              {t(isLoadingModels ? "chat.modelSelector.loading" : "chat.modelSelector.empty")}
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
