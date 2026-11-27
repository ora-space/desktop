import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import {
  Button,
  cn,
  Popover,
  PopoverContent,
  PopoverTrigger,
  StepSlider,
} from "@ora/ui";
import { IconLoader2 } from "@tabler/icons-react";
import {
  currentValueName,
  findThoughtLevelOption,
  thoughtLevelScale,
} from "@ora/chat";
import { useChatStore } from "../../chat-store-context";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useSessions } from "../../state/hooks/use-sessions";
import { useSetSessionConfig } from "../../state/hooks/use-session-config";
import { useAvailableAgents } from "../../state/hooks/use-available-agents";
import { usePendingSwitch } from "../../state/stores/pending-agent-store";

/**
 * The composer's thought-level (reasoning effort) control.
 *
 * Deliberately narrower than the model picker: there is no pre-session catalog
 * for effort levels, and which levels exist depends on the agent — and, for
 * some agents, on the model in effect — so the only trustworthy source is the
 * `thought_level` option the provider session itself reports. The control
 * therefore renders nothing until a conversation carries that option, which
 * happens in exactly two places: the handshake performed by the first send, and
 * the replay of a persisted conversation. A chat that has not started shows no
 * effort control at all rather than guessing one.
 *
 * Effort is an ordered scale rather than a set of unrelated choices, so it is
 * presented as a slider over the levels in the order the agent listed them.
 * Dragging previews the level under the thumb; only releasing commits it, and
 * the commit goes straight through `session/set_config_option`, the same way a
 * model pick on a live session does. The provider answers with its full option
 * set, so a model change that reshapes the available levels is reflected here
 * without this component knowing about models.
 *
 * With an agent move pending, the conversation's options describe the agent it
 * is leaving, so the control withdraws until the move's handshake reports the
 * incoming agent's own levels. It withdraws too once the session is known to have
 * no provider.
 */
export function ThoughtLevelSelector({
  disabled = false,
  sessionId,
}: {
  disabled?: boolean;
  /** Session whose configuration should be displayed instead of workspace selection. */
  sessionId?: string;
}) {
  const { t } = useTranslation();
  // The index the user has put the thumb on but the provider has not confirmed:
  // first while a drag is in progress, then while the commit it ended in is in
  // flight. Held across that round trip on purpose — dropping it at release
  // would snap the thumb back to the last reported value and forward again
  // when the answer lands. `null` means the thumb sits on the reported value.
  const [pendingIndex, setPendingIndex] = useState<number | null>(null);
  // Which commit is the newest. The control stays live during a round trip, so
  // a second pick can overtake a first; only the newest one's answer may
  // release the pending index, or the older answer would snap the thumb back
  // to its own value while the newer request is still out.
  const commitGeneration = useRef(0);
  const selection = useWorkspaceSelectionStore((state) => state.selection);
  const chatStore = useChatStore();
  const setSessionConfig = useSetSessionConfig();
  const { data: sessions = [] } = useSessions();
  const availableAgents = useAvailableAgents();
  // A workflow node names its session explicitly and never carries a pending move.
  const pendingSwitch = usePendingSwitch(
    sessionId === undefined ? selection.sessionId : null,
  );
  const optionsSessionId =
    sessionId ?? (pendingSwitch === undefined ? selection.sessionId : null);
  const boundSession = sessions.find(
    (session) => session.id === optionsSessionId,
  );
  // Options reported before the bound agent's plugin stopped are no longer
  // actionable; an explicitly named workflow session keeps its read-only label.
  const agentIsAvailable =
    sessionId !== undefined ||
    availableAgents.some((agent) => agent.agentRef === boundSession?.agentRef);
  // Selected narrowly so a streaming turn does not re-render the control per token.
  const liveOptions = useStore(chatStore, (state) =>
    optionsSessionId === null
      ? undefined
      : state.conversations[optionsSessionId]?.configOptions,
  );
  const providerLost = useStore(chatStore, (state) =>
    optionsSessionId === null
      ? false
      : state.conversations[optionsSessionId]?.providerLost === true,
  );
  // A session without a provider has no effort to set, and no intent carries one to its next
  // attach, so the control withdraws; a read-only one keeps naming the last level.
  const option =
    liveOptions !== undefined && agentIsAvailable && (disabled || !providerLost)
      ? findThoughtLevelOption(liveOptions)
      : null;
  if (option === null || option.type !== "select") return null;

  const values = thoughtLevelScale(option);
  // A current value off the scale — the agent's `default` — has no rung, so
  // the thumb is withheld and the label falls back to what the agent calls it.
  const currentIndex = values.findIndex(
    (value) => value.value === option.currentValue,
  );
  const shownIndex =
    pendingIndex ?? (currentIndex === -1 ? null : currentIndex);
  const shownLabel =
    (shownIndex === null ? undefined : values[shownIndex]?.name) ??
    currentValueName(option) ??
    option.currentValue;
  const commitLevel = (index: number) => {
    const value = values[index]?.value;
    if (
      value === undefined ||
      value === option.currentValue ||
      optionsSessionId === null
    ) {
      setPendingIndex(null);
      return;
    }
    const generation = (commitGeneration.current += 1);
    setSessionConfig.mutate(
      { sessionId: optionsSessionId, configId: option.id, value },
      // Released only once the provider has spoken: on success the reported
      // value already matches, so the thumb stays put; on failure it returns
      // to the value still in effect, which is the honest position.
      {
        onSettled: () => {
          if (commitGeneration.current === generation) setPendingIndex(null);
        },
      },
    );
  };

  return (
    <Popover>
      <PopoverTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={disabled}
            aria-label={t("chat.thoughtLevel.label")}
            className="h-7 gap-1.5 rounded-md px-2 text-xs font-normal text-muted-foreground hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring/50"
          />
        }
      >
        <span className="whitespace-nowrap">{shownLabel}</span>
      </PopoverTrigger>
      <PopoverContent align="end" side="top" className="w-64 gap-1 px-3.5 py-3">
        <div className="flex items-center gap-2 text-sm">
          <span className="text-muted-foreground">
            {t("chat.thoughtLevel.title")}
          </span>
          <span className="font-medium">{shownLabel}</span>
          {/* Lives here in a slot that is always laid out, never on the
              trigger: a spinner appearing beside the trigger label would
              widen the button and shove the bar's contents sideways for the
              length of the round trip. */}
          <IconLoader2
            className={cn(
              "ml-auto size-3.5 shrink-0 animate-spin text-muted-foreground",
              !setSessionConfig.isPending && "invisible",
            )}
            aria-hidden="true"
          />
        </div>
        {/* The two ends of the scale are named in terms of what moving toward
            them trades, since the agent's own level names say nothing about
            what a higher one costs. */}
        <div className="flex justify-between text-xs text-muted-foreground">
          <span>{t("chat.thoughtLevel.faster")}</span>
          <span>{t("chat.thoughtLevel.smarter")}</span>
        </div>
        {/* Deliberately not disabled while a commit is in flight: dimming the
            control for the round trip reads as a flicker, and a newer pick
            simply overtakes the older request. */}
        <StepSlider
          aria-label={t("chat.thoughtLevel.title")}
          value={shownIndex}
          stepCount={values.length}
          onValueChange={setPendingIndex}
          onValueCommitted={commitLevel}
        />
      </PopoverContent>
    </Popover>
  );
}
