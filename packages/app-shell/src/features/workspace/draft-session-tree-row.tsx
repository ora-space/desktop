import { memo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useStoreWithEqualityFn } from "zustand/traditional";
import { Button } from "@ora/ui";
import { IconMessageCircle, IconX } from "@tabler/icons-react";
import {
  draftSidebarTitle,
  useDraftSessionsStore,
} from "../../state/stores/draft-sessions-store";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import {
  dismissSessionDraft,
  selectBoundDraftSession,
} from "../../state/session-drafts";

interface DraftSessionTreeRowProps {
  draftId: string;
  depth: 0 | 1 | 2;
}

type DraftRowSnapshot = {
  id: string;
  text: string;
  projectId: string;
  taskId: string | null;
  pendingSessionId: string | null;
  sendInFlight: boolean;
};

/** Limits re-renders to rows whose own draft fields changed. */
function draftRowSnapshotEqual(
  left: DraftRowSnapshot | null,
  right: DraftRowSnapshot | null,
): boolean {
  if (left === right) return true;
  if (left === null || right === null) return false;
  return (
    left.id === right.id &&
    left.text === right.text &&
    left.projectId === right.projectId &&
    left.taskId === right.taskId &&
    left.pendingSessionId === right.pendingSessionId &&
    left.sendInFlight === right.sendInFlight
  );
}

/**
 * Muted new-chat leaf. It stays visually quiet until attach turns it into a
 * real session. Bound rows open that session; unbound ones restore the
 * composer. × stays hidden while bound or while sendInFlight so an in-flight
 * first send cannot be yanked away before repark can land.
 */
export const DraftSessionTreeRow = memo(function DraftSessionTreeRow({
  draftId,
  depth,
}: DraftSessionTreeRowProps) {
  const { t } = useTranslation();
  const rowRef = useRef<HTMLDivElement>(null);
  const draft = useStoreWithEqualityFn(
    useDraftSessionsStore,
    (s) => {
      const candidate = s.drafts.find((entry) => entry.id === draftId);
      if (candidate === undefined) return null;
      return {
        id: candidate.id,
        text: candidate.text,
        projectId: candidate.projectId,
        taskId: candidate.taskId,
        pendingSessionId: candidate.pendingSessionId,
        sendInFlight: candidate.sendInFlight,
      };
    },
    draftRowSnapshotEqual,
  );
  const pendingSessionId = draft?.pendingSessionId ?? null;
  // O(1) compare so selection changes do not scan the drafts array per row.
  // Boolean result keeps inactive rows from re-rendering when selection moves.
  const active = useWorkspaceSelectionStore(
    (s) =>
      s.selection.draftId === draftId ||
      (pendingSessionId !== null && s.selection.sessionId === pendingSessionId),
  );
  if (draft === null) return null;

  const bound = pendingSessionId !== null;
  const title = draftSidebarTitle(draft.text, t("sidebar.newSession"));
  const current = draft;

  /** Bound drafts jump to the live session; others restore the parked composer. */
  function handleSelect() {
    if (current.pendingSessionId !== null) {
      selectBoundDraftSession({
        projectId: current.projectId,
        taskId: current.taskId,
        pendingSessionId: current.pendingSessionId,
      });
      return;
    }
    useWorkspaceSelectionStore
      .getState()
      .selectDraft(current.id, current.taskId, current.projectId);
    useUiStore.getState().expandProject(current.projectId);
    if (current.taskId !== null) {
      useUiStore.getState().expandTask(current.taskId);
    }
  }

  return (
    <div
      ref={rowRef}
      className={`group/tree flex h-9 items-center rounded-md transition-colors ${
        active
          ? "bg-sidebar-accent text-sidebar-accent-foreground"
          : "hover:bg-sidebar-accent/70"
      }`}
    >
      <div
        role="button"
        tabIndex={0}
        onClick={handleSelect}
        onKeyDown={(event) => {
          if (event.key !== "Enter" && event.key !== " ") return;
          event.preventDefault();
          handleSelect();
        }}
        className="flex h-full min-w-0 flex-1 cursor-pointer items-center gap-2 rounded-md text-left text-[13px] outline-none focus-visible:ring-2 focus-visible:ring-ring"
        style={{ paddingLeft: `${8 + depth * 18}px` }}
      >
        <span className="flex size-[18px] shrink-0 items-center justify-center opacity-60">
          <IconMessageCircle
            className="size-4 text-muted-foreground"
            aria-hidden="true"
          />
        </span>
        <span className="min-w-0 flex-1 truncate font-medium text-muted-foreground">
          {title}
        </span>
      </div>
      {!bound && !current.sendInFlight && (
        <div
          className={`mr-1 flex items-center transition-opacity duration-100 ${
            active
              ? "opacity-100"
              : "opacity-0 group-hover/tree:opacity-100 group-focus-within/tree:opacity-100"
          }`}
        >
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={t("sidebar.dismissDraft")}
            onClick={(event) => {
              event.stopPropagation();
              const navigation = rowRef.current?.closest("nav");
              const focusableRows = navigation
                ? [
                    ...navigation.querySelectorAll<HTMLElement>(
                      '[role="button"][tabindex="0"]',
                    ),
                  ]
                : [];
              const currentIndex = focusableRows.findIndex((row) =>
                rowRef.current?.contains(row),
              );
              const nextFocus =
                focusableRows[currentIndex + 1] ??
                focusableRows[currentIndex - 1] ??
                null;
              dismissSessionDraft(current.id);
              // The clicked × disappears with the draft. Restore keyboard
              // position to an adjacent row instead of falling back to body.
              queueMicrotask(() => nextFocus?.focus());
            }}
          >
            <IconX />
          </Button>
        </div>
      )}
    </div>
  );
});
