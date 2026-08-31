import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
  Input,
} from "@ora/ui";
import {
  IconCheck,
  IconChevronDown,
  IconChevronRight,
  IconLoader2,
  IconPlug,
  IconRobot,
  IconSearch,
} from "@tabler/icons-react";
import { useChatStore } from "../../chat-store-context";
import { useSettingsStore } from "../../state/stores/settings-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useUiStore } from "../../state/stores/ui-store";
import { useSessions } from "../../state/hooks/use-sessions";
import { useSetSessionConfig } from "../../state/hooks/use-session-config";
import {
  useWarmSession,
  warmTargetKey,
} from "../../state/hooks/use-warm-session";
import { useTargetAgentCli } from "../../state/hooks/use-target-agent-cli";
import { useAvailableAgents } from "../../state/hooks/use-available-agents";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";
import {
  usePendingAgentStore,
  usePendingSwitch,
} from "../../state/stores/pending-agent-store";
import { useAgentModelStore } from "../../state/stores/agent-model-store";
import { currentValueName, findModelOption, selectableValues } from "@ora/chat";
import { PluginLogoMark } from "../settings/plugin-logo";

/**
 * The composer's agent and model picker.
 *
 * Both lists describe the session the composer will send into. Which agents exist
 * is whatever this installation can actually reach; which models are available is
 * whatever the chosen agent reported for this session, so the model list has three
 * states rather than two — still arriving, genuinely offering no choice, or a real
 * set to pick from.
 *
 * With a session selected, choosing a different agent moves that conversation onto
 * it rather than only changing the default for the next one. Ora owns the
 * transcript, so the thread survives the move: the backend hands it to the new
 * agent with the user's next message. The move is *recorded* here and performed
 * by that message — clicking cannot rebind immediately without tearing down an
 * agent that may still be mid-reply. What the click does start is warming the
 * incoming CLI, which is how the models below can be replaced by its own before
 * anything is committed. Choosing a CLI therefore leaves the menu open: picking
 * one of those models is the other half of the same decision.
 */
export function ModelSelector({
  disabled = false,
  sessionId,
}: {
  disabled?: boolean;
  /** Session whose model configuration should be displayed instead of workspace selection. */
  sessionId?: string;
}) {
  const { t } = useTranslation();
  // Local to the picker rather than persisted: it is a scratch filter on
  // whatever the current model list is, not a preference worth remembering
  // once the menu closes.
  const [modelQuery, setModelQuery] = useState("");
  const updateSettings = useSettingsStore((state) => state.updateSettings);
  const selection = useWorkspaceSelectionStore((state) => state.selection);
  const chatStore = useChatStore();
  const setSessionConfig = useSetSessionConfig();
  const { data: sessions = [] } = useSessions();
  // Workflow node sessions live inside the run workspace without becoming the global workspace
  // selection. Binding the picker explicitly keeps its read-only label attached to that node.
  const modelSelection =
    sessionId === undefined ? selection : { ...selection, sessionId };

  // Having a binding is what makes a session persisted, and only a persisted one
  // can be rebound; a warm session has no row to move. The bound CLI is also what
  // a candidate has to be compared against to decide whether picking it is a move
  // at all — the resolved agent below cannot answer that, since it already
  // reports whatever move is pending.
  const boundSession = sessions.find(
    (session) => session.id === modelSelection.sessionId,
  );
  const targetKey = warmTargetKey(modelSelection);
  const setPickedForTarget = usePendingAgentStore(
    (state) => state.setPendingAgent,
  );
  const setPendingSwitch = usePendingAgentStore(
    (state) => state.setPendingSwitch,
  );
  const clearPendingSwitch = usePendingAgentStore(
    (state) => state.clearPendingSwitch,
  );
  const pendingSwitch = usePendingSwitch(modelSelection.sessionId);
  // Resolved centrally so this and the composer cannot disagree: they share one
  // warm-session query key, and the CLI is part of that key.
  const agentCli = useTargetAgentCli(modelSelection);
  // Which agents the runtime actually reports reaching here. An agent whose
  // plugin package was uninstalled, or whose own agent process is missing,
  // drops out of the list rather than being offered and then failing on the
  // first message.
  const availableAgents = useAvailableAgents();
  // Whether this installation has *any* agent plugin installed at all. This is
  // broader than availability: an installed agent can still be unreachable (a
  // disabled package, a missing runtime) and must not read as "install one" —
  // the user already has it. Distinguishing "no agent plugin ever installed"
  // from "installed but unavailable" is what lets the picker offer the install
  // hint only to a truly empty installation.
  const { data: installedPlugins, isPending: pluginsPending } =
    useInstalledPlugins();
  const noAgentPackageInstalled =
    installedPlugins !== undefined &&
    installedPlugins.every((plugin) => plugin.kind !== "agent");
  // Opening the marketplace is a one-shot intent; the loading gate keeps the hint
  // from flashing "go install one" while the installed snapshot is still in flight.
  const openPluginMarketplace = () => {
    if (pluginsPending || !noAgentPackageInstalled) return;
    useUiStore.getState().openSettingsAt("plugins");
  };
  // Preserve the internal preference across temporary unavailability without
  // presenting that unavailable runtime as the active picker identity.
  const displayedAgent = availableAgents.find(
    (agent) => agent.agentRef === agentCli,
  );
  const agentIsAvailable = displayedAgent !== undefined;

  // Shares the workspace's warm-session query key, so this is a cache read
  // rather than a second provider session.
  const warmSession = useWarmSession(modelSelection, agentCli);
  // A warm session, when there is one, always describes the CLI on screen —
  // including the one a pending move is heading for, whose models and model
  // choice live on it rather than on the session being moved. While that
  // handshake is still running there is nothing to read: naming the bound
  // session here instead would advertise the outgoing agent's model as the
  // incoming agent's.
  const activeSessionId =
    warmSession.sessionId ??
    (pendingSwitch === undefined ? modelSelection.sessionId : null);
  // Selected narrowly rather than as one conversation object, so a streaming
  // turn does not re-render the picker on every token.
  const liveOptions = useStore(chatStore, (state) =>
    activeSessionId === null
      ? undefined
      : state.conversations[activeSessionId]?.configOptions,
  );
  const isReplayingHistory = useStore(chatStore, (state) =>
    activeSessionId === null
      ? false
      : state.conversations[activeSessionId]?.isLoading === true,
  );
  // What this CLI reported the last time any surface handshook it. Standing in
  // for an answer that has not arrived is the whole point: the list barely
  // changes between sessions, and waiting for `session/new` to say so again is
  // what made opening a chat feel slow.
  const cachedOptions = useAgentModelStore((state) =>
    agentCli === null ? undefined : state.known[agentCli],
  );
  // A session can retain the last options reported before its plugin stopped.
  // They are no longer actionable once runtime availability drops, so do not
  // let that session-local snapshot outlive the agent row that owned it.
  // Workflow node conversations are scoped by an explicit session id and may
  // not appear in the workspace session list. Their read-only picker must keep
  // showing the model captured by that conversation even when no workspace
  // Agent preference can be resolved for it.
  const configOptions =
    sessionId !== undefined
      ? liveOptions
      : agentIsAvailable
        ? (liveOptions ?? cachedOptions)
        : undefined;
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
  const isSettling =
    agentIsAvailable &&
    (activeSessionId === null
      ? warmSession.isOpening || pendingSwitch !== undefined
      : liveOptions === undefined || isReplayingHistory);
  // Having a list to offer is what ends the wait, not having received this
  // surface's own answer: a cached list is a real answer to "what can I pick",
  // and showing it beats spinning while the handshake confirms it. Gating on the
  // resolved option rather than on the options array is deliberate — a replaying
  // session is seeded with an empty one, which is a placeholder rather than a
  // list, and must keep reading as "still arriving".
  const isLoadingModels = isSettling && modelOption === null;

  // A cached list describes the CLI, not a session, so there is nothing to write
  // a choice from it to until the handshake produces one. The values are shown
  // but not selectable for that window rather than hidden, because what the user
  // is waiting to learn — which models this agent has — is already answered.
  const canSelectModel = agentIsAvailable && activeSessionId !== null;

  // The disabled cached list on its own reads as settled, not provisional — the
  // handshake could still replace it with a different set. This names that
  // in-between case so the dropdown can say so, distinct from isLoadingModels
  // (nothing to show yet) even though both are the same underlying wait.
  const isUpdatingModels = isSettling && modelOption !== null;

  const activeLabel = modelOption
    ? currentValueName(modelOption)
    : t(
        isLoadingModels
          ? "chat.modelSelector.loading"
          : "chat.modelSelector.placeholder",
      );

  /**
   * A persisted session records a move onto the chosen CLI, to be performed by
   * the next message sent into it; a not-yet-started chat records the pick
   * against its own target instead, so it survives navigating away and back
   * without touching any other chat.
   *
   * Picking the CLI a session is still bound to withdraws the recorded move
   * instead of recording one onto it. Nothing was rebound when the other CLI was
   * chosen, so arriving back at the bound one leaves the conversation exactly
   * where it started — and asking the backend to rebind a session onto the agent
   * it already runs on is refused as `session_agent_unchanged`.
   *
   * Either way the shared default also moves, so the next chat surface no one
   * has touched yet still opens on whatever the user picked most recently, and
   * either way the CLI's own models arrive from a handshake that has not
   * happened yet — this only points the surface at it. The list below settles
   * when that answers, which is why the menu is still open to see it.
   */
  const selectAgent = (candidate: string) => {
    if (candidate === agentCli) return;
    updateSettings({ agentCli: candidate });
    if (boundSession !== undefined) {
      if (candidate === boundSession.agentRef) {
        clearPendingSwitch(boundSession.id);
      } else {
        setPendingSwitch(boundSession.id, candidate);
      }
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

  const modelValues = modelOption ? selectableValues(modelOption) : [];
  const needle = modelQuery.trim().toLowerCase();
  const visibleModelValues = needle
    ? modelValues.filter((value) => value.name.toLowerCase().includes(needle))
    : modelValues;

  return (
    <DropdownMenu onOpenChange={(open) => !open && setModelQuery("")}>
      <DropdownMenuTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={disabled}
            aria-label={t("chat.modelSelector.label")}
            className="group/model h-7 gap-1.5 rounded-md px-2 text-xs font-normal text-muted-foreground hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring/50"
          />
        }
      >
        {displayedAgent && (
          <PluginLogoMark
            logo={displayedAgent.logo}
            fallback={IconRobot}
            className="size-3.5 shrink-0 object-contain"
          />
        )}
        {/* The CLI name is width-animated in via a 0fr → 1fr grid so the
            button grows smoothly on hover instead of snapping wider. */}
        <span className="grid grid-cols-[0fr] opacity-0 transition-all duration-200 group-hover/model:grid-cols-[1fr] group-hover/model:opacity-100 group-aria-expanded/model:grid-cols-[1fr] group-aria-expanded/model:opacity-100">
          <span className="min-w-0 overflow-hidden whitespace-nowrap">
            {displayedAgent?.label ?? ""}
          </span>
        </span>
        <span className="whitespace-nowrap">{activeLabel}</span>
        {setSessionConfig.isPending || isSettling ? (
          <IconLoader2
            className="size-3 shrink-0 animate-spin opacity-50"
            aria-hidden="true"
          />
        ) : (
          <IconChevronDown
            className="size-3 shrink-0 opacity-50"
            aria-hidden="true"
          />
        )}
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        side="top"
        className="w-56 max-h-[min(24rem,var(--available-height))]"
      >
        {noAgentPackageInstalled ? (
          <DropdownMenuGroup className="p-1">
            <DropdownMenuLabel className="px-2 py-1.5 text-xs font-normal text-muted-foreground">
              {t("chat.modelSelector.agent")}
            </DropdownMenuLabel>
            <DropdownMenuItem
              className="flex items-center gap-1.5 rounded-sm px-2 py-2 text-xs"
              // Navigating to the marketplace closes the menu so the settings
              // surface is not fighting a still-open popover.
              onClick={openPluginMarketplace}
            >
              <IconPlug className="size-3.5 shrink-0" aria-hidden="true" />
              <span className="min-w-0 flex-1 text-left text-muted-foreground">
                {t("chat.modelSelector.noAgentPackage")}
              </span>
              <IconChevronRight className="size-3.5 shrink-0 opacity-50" />
            </DropdownMenuItem>
          </DropdownMenuGroup>
        ) : (
          <>
            <DropdownMenuGroup className="p-1">
              <DropdownMenuLabel className="px-2 py-1.5 text-xs font-normal text-muted-foreground">
                {t("chat.modelSelector.agent")}
              </DropdownMenuLabel>
              {availableAgents.map((candidate) => (
                <DropdownMenuItem
                  key={candidate.agentRef}
                  className="gap-1.5 rounded-sm px-2 py-1.5 text-xs"
                  // Choosing an agent is only half the choice: its models replace the
                  // group below and the user still has to pick one from them.
                  closeOnClick={false}
                  onClick={() => selectAgent(candidate.agentRef)}
                >
                  <PluginLogoMark
                    logo={candidate.logo}
                    fallback={IconRobot}
                    className="size-3.5 object-contain"
                  />
                  {candidate.label}
                  {candidate.agentRef === agentCli && (
                    <IconCheck className="ml-auto size-4" />
                  )}
                </DropdownMenuItem>
              ))}
            </DropdownMenuGroup>
            <DropdownMenuGroup className="p-1">
              <DropdownMenuLabel className="flex items-center gap-1 px-2 py-1.5 text-xs font-normal text-muted-foreground">
                {t("chat.modelSelector.model")}
                {isUpdatingModels && (
                  <span className="inline-flex items-center gap-1 text-muted-foreground/70">
                    <IconLoader2
                      className="size-3 animate-spin"
                      aria-hidden="true"
                    />
                    {t("chat.modelSelector.updating")}
                  </span>
                )}
              </DropdownMenuLabel>
              {/* Kept out of the agent group above: the agent list is short and
                  unsearched, and a query here should never be mistaken for
                  filtering which CLI is offered. */}
              {modelValues.length > 0 && (
                <div className="relative px-0.5 pb-1">
                  <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
                  <Input
                    value={modelQuery}
                    onChange={(event) => setModelQuery(event.target.value)}
                    placeholder={t("chat.modelSelector.search")}
                    aria-label={t("chat.modelSelector.search")}
                    className="h-7 border-transparent bg-muted/50 pl-8 text-xs shadow-none focus-visible:ring-1 focus-visible:ring-ring/50"
                    // Excluded from the tab order so the menu's own focus manager,
                    // which moves focus to the popup's first tabbable descendant
                    // on open, does not land here — opening the menu should not
                    // steal focus into the search box. A click still focuses it.
                    tabIndex={-1}
                    // The dropdown's own typeahead and arrow-key navigation listen
                    // on the popup element, so unfiltered keystrokes here would be
                    // swallowed as menu navigation instead of reaching the input.
                    onKeyDown={(event) => event.stopPropagation()}
                    onClick={(event) => event.stopPropagation()}
                  />
                </div>
              )}
              {modelOption === null ? (
                <p className="px-2 py-4 text-center text-xs text-muted-foreground">
                  {t(
                    isLoadingModels
                      ? "chat.modelSelector.loading"
                      : "chat.modelSelector.empty",
                  )}
                </p>
              ) : visibleModelValues.length === 0 ? (
                <p className="px-2 py-4 text-center text-xs text-muted-foreground">
                  {t("chat.modelSelector.noResults")}
                </p>
              ) : (
                visibleModelValues.map((value) => (
                  <DropdownMenuItem
                    key={value.value}
                    className="gap-1.5 rounded-sm px-2 py-1.5 text-xs"
                    disabled={!canSelectModel}
                    onClick={() => selectModel(value.value)}
                  >
                    {value.name}
                    {modelOption.type === "select" &&
                      value.value === modelOption.currentValue && (
                        <IconCheck className="ml-auto size-4" />
                      )}
                  </DropdownMenuItem>
                ))
              )}
            </DropdownMenuGroup>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
