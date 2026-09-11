import {
  IconAlertTriangle,
  IconBan,
  IconInfoCircle,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import type { ChatTurn } from "@ora/chat";
import { ActivityPhase } from "./activity-phase";
import { MessageBubble } from "./message-bubble";
import { ContentBlock } from "./content-block";
import {
  buildTurnDisplayItems,
  type DisplayTurnItem,
} from "./turn-item-grouping";
import { TurnDiffSummary } from "./turn-diff-summary";

interface ResponseTurnProps {
  turn: ChatTurn;
  userName: string;
}

/** Groups all agent activity for one prompt under a single assistant identity. */
export function ResponseTurn({ turn, userName }: ResponseTurnProps) {
  const { t } = useTranslation();
  const displayItems = buildTurnDisplayItems(turn.items, turn.status);
  return (
    <section className="py-3" aria-label={t("chat.assistantReplied")}>
      <div className="min-w-0 space-y-2.5">
        {displayItems.map((item, index) => (
          <DisplayTurnItemView
            key={item.id}
            item={item}
            turn={turn}
            userName={userName}
            displayIndex={index}
            displayCount={displayItems.length}
          />
        ))}
        <TurnEnding turn={turn} />
        <TurnDiffSummary turn={turn} />
      </div>
    </section>
  );
}

/** Renders one grouped response block so the thread list can window it independently. */
export function DisplayTurnItemView({
  item,
  turn,
  userName,
  displayIndex,
  displayCount,
}: {
  item: DisplayTurnItem;
  turn: ChatTurn;
  userName: string;
  displayIndex: number;
  displayCount: number;
}) {
  switch (item.kind) {
    case "activityPhase":
      return (
        <ActivityPhase
          phase={item}
          turnStatus={turn.status}
          isLatestActivity={displayIndex === displayCount - 1}
        />
      );
    case "message":
      return (
        <MessageBubble
          message={item}
          userName={userName}
          embeddedAssistant
          streaming={
            turn.status === "streaming" && displayIndex === displayCount - 1
          }
        />
      );
    case "content":
      return <ContentBlock content={item.content} />;
  }
}

/** Explains non-standard turn endings without treating them as transport failures. */
export function TurnEnding({ turn }: { turn: ChatTurn }) {
  const { t } = useTranslation();
  if (turn.status === "cancelled") {
    return (
      <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <IconBan className="size-3.5" />
        {t("chat.turnCancelled")}
      </p>
    );
  }
  if (turn.status === "failed") {
    return (
      <p
        data-selectable
        className="flex items-center gap-1.5 text-xs text-destructive"
      >
        <IconAlertTriangle className="size-3.5" />
        {turn.error ?? t("chat.turnFailed")}
      </p>
    );
  }
  if (
    turn.stopReason === "max_tokens" ||
    turn.stopReason === "max_turn_requests"
  ) {
    return (
      <p className="flex items-center gap-1.5 text-xs text-amber-700 dark:text-amber-400">
        <IconAlertTriangle className="size-3.5" />
        {t("chat.turnIncomplete")}
      </p>
    );
  }
  if (turn.stopReason === "refusal") {
    return (
      <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <IconInfoCircle className="size-3.5" />
        {t("chat.turnRefused")}
      </p>
    );
  }
  return null;
}
