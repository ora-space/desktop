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
  chatSurfaceTargetKey,
  useTargetAgentCli,
} from "../../state/hooks/use-target-agent-cli";
import { useAvailableAgents } from "../../state/hooks/use-available-agents";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";
import {
  usePendingAgentStore,
  pendingModelKey,
  usePendingSwitch,
} from "../../state/stores/pending-agent-store";
import { currentValueName, findModelOption, selectableValues } from "@ora/chat";
import { PluginLogoMark } from "../settings/plugin-logo";
import { useAgentModels } from "../../state/hooks/use-agent-models";
import { useTasks } from "../../state/hooks/use-tasks";
import { useWorkspaces } from "../../state/hooks/use-workspaces";

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
 * agent that may still be mid-reply. Model discovery for the incoming CLI runs
 * independently, so the models below can be replaced by its own before
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
  // can be rebound. The bound CLI is also what
  // a candidate has to be compared against to decide whether picking it is a move
  // at all — the resolved agent below cannot answer that, since it already
  // reports whatever move is pending.
  const boundSession = sessions.find(
    (session) => session.id === modelSelection.sessionId,
  );
  const targetKey = chatSurfaceTargetKey(modelSelection);
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
  // Resolved centrally so this and the composer cannot disagree about the target agent.
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

  const { data: tasks = [] } = useTasks();
  const { data: workspaces = [] } = useWorkspaces();
  const workspaceId =
    tasks.find((task) => task.id === modelSelection.taskId)?.workspaceId ??
    workspaces.find(
      (workspace) =>
        workspace.projectId === modelSelection.projectId &&
        workspace.kind === "main",
    )?.id ??
    null;
  const usesPersistedOptions =
    sessionId !== undefined ||
    (boundSession !== undefined && pendingSwitch === undefined);
  const discovered = useAgentModels(
    usesPersistedOptions ? null : agentCli,
    usesPersistedOptions ? null : workspaceId,
  );
  const activeSessionId = usesPersistedOptions
    ? modelSelection.sessionId
    : null;
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
  // A session can retain the last options reported before its plugin stopped.
  // They are no longer actionable once runtime availability drops, so do not
  // let that session-local snapshot outlive the agent row that owned it.
  // Workflow node conversations are scoped by an explicit session id and may
  // not appear in the workspace session list. Their read-only picker must keep
  // showing the model captured by that conversation even when no workspace
  // Agent preference can be resolved for it.
  const configOptions =
    sessionId !== undefined || agentIsAvailable ? liveOptions : undefined;
  const modelOption = configOptions ? findModelOption(configOptions) : null;
  // Discovery answers for one agent ref and stays cached after that agent drops
  // out of the runtime. Withholding it here mirrors `configOptions` above: a
  // catalog belonging to an unreachable agent must not outlive its row.
  const discoveredModels = agentIsAvailable ? discovered.models : [];
  const intentKey =
    agentCli === null ? null : pendingModelKey(modelSelection, agentCli);
  const pendingModel = usePendingAgentStore((state) =>
    intentKey === null ? undefined : state.models[intentKey],
  );
  const setPendingModel = usePendingAgentStore(
    (state) => state.setPendingModel,
  );
  const modelValues = usesPersistedOptions
    ? modelOption
      ? selectableValues(modelOption).map((value) => ({
          value: value.value,
          name: value.name,
        }))
      : []
    : discoveredModels.map((model) => ({
        value: model.id,
        name: model.displayName,
      }));
  // A persisted session is authoritative about the model it is running on. A
  // chat that has not started yet has only what the user recorded here, falling
  // back to whatever the agent nominates as its own default so the picker names
  // the model the first send will actually ask for.
  const discoveredDefault =
    discoveredModels.find((model) => model.default)?.id ??
    discoveredModels[0]?.id;
  const selectedValue = usesPersistedOptions
    ? modelOption?.type === "select"
      ? modelOption.currentValue
      : undefined
    : (pendingModel ?? discoveredDefault);
  // Three states rather than two, because "not answered yet" must not read as
  // "this agent offers no models". A persisted session has said nothing until
  // its conversation is loaded — and replay seeds empty options first, which is
  // a placeholder and not a list — while a not-yet-started chat is waiting on
  // discovery instead.
  const isSettling = usesPersistedOptions
    ? liveOptions === undefined || isReplayingHistory
    : discovered.isLoading;
  const isLoadingModels = isSettling && modelValues.length === 0;
  // A list already on screen being refreshed is its own case: the menu says so
  // rather than blanking what the user is reading.
  const isUpdatingModels = discovered.isFetching && discoveredModels.length > 0;
  const activeLabel =
    usesPersistedOptions && modelOption
      ? currentValueName(modelOption)
      : (modelValues.find((value) => value.value === selectedValue)?.name ??
        t(
          isLoadingModels
            ? "chat.modelSelector.loading"
            : "chat.modelSelector.placeholder",
        ));
  // Applying a choice needs somewhere to put it: a persisted session needs the
  // option the agent reported, and a not-yet-started chat needs a target key to
  // record the intent against.
  const canSelectModel = usesPersistedOptions
    ? agentIsAvailable && activeSessionId !== null && modelOption !== null
    : agentIsAvailable && intentKey !== null;

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
    if (!usesPersistedOptions) {
      if (intentKey !== null) setPendingModel(intentKey, value);
      return;
    }
    if (activeSessionId !== null && modelOption !== null) {
      setSessionConfig.mutate({
        sessionId: activeSessionId,
        configId: modelOption.id,
        value,
      });
    }
  };

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
              {modelValues.length === 0 ? (
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
                    {value.value === selectedValue && (
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
