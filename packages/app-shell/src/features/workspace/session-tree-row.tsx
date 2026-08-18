import { memo } from "react";
import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import {
  Button,
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
  Input,
  toast,
} from "@ora/ui";
import {
  IconArchive,
  IconAlertTriangle,
  IconMessageCircle,
  IconPencil,
  IconTrash,
} from "@tabler/icons-react";
import type { TaskWorkspaceMode } from "@ora/contracts";
import { AgentActivityDots } from "../../components/agent-activity-dots";
import { useChatStore } from "../../chat-store-context";
import { useRenameSession } from "../../state/hooks/use-workspace-mutations";
import { useUiStore } from "../../state/stores/ui-store";
import { useUnreadSessionsStore } from "../../state/stores/unread-sessions-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useInlineTreeRename } from "./use-inline-tree-rename";

interface SessionTreeRowProps {
  sessionId: string;
  taskId: string;
  projectId: string;
  title: string;
  depth: 0 | 1 | 2;
  /** Direct chats delete the wrapping task; worktree leaves delete the session. */
  deleteAs: "session" | "task";
  workspaceMode?: TaskWorkspaceMode;
}

/**
 * One session leaf with hover archive stub, right-click actions, and inline rename.
 *
 * Activity/unread/selection are read here so chat tokens never re-render the rest of
 * the tree. The trigger is a `div` (via `render`) so WebKit/Tauri deliver `contextmenu`.
 * Archive is a placeholder until persistence ships.
 */
export const SessionTreeRow = memo(function SessionTreeRow({
  sessionId,
  taskId,
  projectId,
  title,
  depth,
  deleteAs,
  workspaceMode,
}: SessionTreeRowProps) {
  const { t } = useTranslation();
  const renameSession = useRenameSession();
  const selectSession = useWorkspaceSelectionStore((s) => s.selectSession);
  const setDeleteTarget = useUiStore((s) => s.setDeleteTarget);
  const active = useWorkspaceSelectionStore(
    (s) => s.selection.sessionId === sessionId,
  );
  const unread = useUnreadSessionsStore((s) => s.unread.has(sessionId));
  const chatStore = useChatStore();
  const permissionRequired = useStore(chatStore, (s) =>
    Boolean(s.conversations[sessionId]?.pendingPermissions.length),
  );
  const isResponding = useStore(chatStore, (s) =>
    Boolean(s.conversations[sessionId]?.isResponding),
  );
  const {
    renaming,
    draft,
    setDraft,
    inputRef,
    restoreMenuFocus,
    beginRename,
    onInputKeyDown,
    onInputBlur,
    maxLength,
  } = useInlineTreeRename({
    value: title,
    onCommit: (next) => renameSession.mutateAsync({ sessionId, title: next }),
  });

  /** Selects this session without depending on a parent callback identity. */
  function handleSelect() {
    selectSession(sessionId, taskId, projectId);
  }

  /** Opens the existing delete confirmation, using the visible session title. */
  function handleDelete() {
    if (deleteAs === "task") {
      setDeleteTarget({
        kind: "task",
        id: taskId,
        name: title,
        workspaceMode: workspaceMode ?? "project_root",
        sessionIds: [sessionId],
      });
      return;
    }
    setDeleteTarget({ kind: "session", id: sessionId, name: title });
  }

  /** Archive persistence is not shipped; keep the control visible with feedback. */
  function handleArchive() {
    toast(t("sidebar.archiveSoon"));
  }

  const icon = permissionRequired ? (
    <IconAlertTriangle
      className="size-[18px] text-amber-500"
      aria-label={t("sidebar.permissionRequired")}
    />
  ) : isResponding ? (
    <AgentActivityDots
      label={t("common.running")}
      className="text-muted-foreground"
    />
  ) : unread ? (
    <UnreadDot label={t("sidebar.unread")} />
  ) : (
    <IconMessageCircle
      className="size-4 text-muted-foreground"
      aria-label={
        deleteAs === "task" ? t("sidebar.directChatTask") : t("sidebar.session")
      }
    />
  );

  return (
    <ContextMenu>
      <ContextMenuTrigger
        render={
          <div
            className={`group/tree flex h-9 items-center rounded-md transition-colors ${
              active
                ? "bg-sidebar-accent text-sidebar-accent-foreground"
                : "hover:bg-sidebar-accent/70"
            }`}
            onContextMenu={(event) => event.preventDefault()}
          />
        }
      >
        {renaming ? (
          <div
            className="flex h-full min-w-0 flex-1 items-center gap-2"
            style={{ paddingLeft: `${8 + depth * 18}px` }}
          >
            <span className="flex size-[18px] shrink-0 items-center justify-center">
              {icon}
            </span>
            <Input
              ref={inputRef}
              value={draft}
              maxLength={maxLength}
              aria-label={t("sidebar.rename")}
              className="h-7 flex-1 border-transparent bg-background px-1.5 text-[13px] shadow-none"
              onChange={(event) => setDraft(event.target.value)}
              onClick={(event) => event.stopPropagation()}
              onKeyDown={onInputKeyDown}
              onBlur={onInputBlur}
            />
          </div>
        ) : (
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
            <span className="flex size-[18px] shrink-0 items-center justify-center">
              {icon}
            </span>
            <span className="min-w-0 flex-1 truncate font-medium">{title}</span>
          </div>
        )}
        {!renaming && (
          <div className="mr-1 flex items-center opacity-0 transition-opacity duration-100 group-hover/tree:opacity-100 group-focus-within/tree:opacity-100">
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={t("sidebar.archive")}
              onClick={(event) => {
                event.stopPropagation();
                handleArchive();
              }}
            >
              <IconArchive />
            </Button>
          </div>
        )}
      </ContextMenuTrigger>
      {/* Rename suppresses restore so the editor keeps focus; other actions still return it. */}
      <ContextMenuContent
        className="w-44"
        finalFocus={() => restoreMenuFocus.current}
      >
        <ContextMenuItem onClick={beginRename}>
          <IconPencil />
          {t("sidebar.rename")}
        </ContextMenuItem>
        <ContextMenuItem onClick={handleArchive}>
          <IconArchive />
          {t("sidebar.archive")}
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem variant="destructive" onClick={handleDelete}>
          <IconTrash />
          {t("common.delete")}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
});

/** Marks a session that finished a turn the user has not opened yet. */
function UnreadDot({ label }: { label: string }) {
  return (
    <span
      role="img"
      aria-label={label}
      className="size-2 rounded-full bg-sky-500"
    />
  );
}
