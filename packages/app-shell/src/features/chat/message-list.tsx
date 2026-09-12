import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  selectElementContents,
  TextEditContextMenu,
  writeClipboardText,
} from "../editor/text-edit-context-menu";
import {
  ConversationNavigator,
  type ConversationNavigationPresentation,
} from "./conversation-navigator";
import { useConversationNavigation } from "./conversation-navigation";
import type { ChatModelChange, ChatTurn } from "@ora/chat";
import { useTaskWorkspace } from "../../state/hooks/use-task-workspace";
import { useWorkspaceCwd } from "../../state/hooks/use-workspace-cwd";
import {
  collectCumulativeArtifactIndices,
  type TurnArtifactCacheEntry,
} from "./chat-link/artifact-index";
import {
  ChatLinkContext,
  type ChatLinkContextValue,
} from "./chat-link/context";
import { MessageListRowView } from "./message-list-row";
import {
  buildMessageListRows,
  estimateMessageListRowSize,
  MESSAGE_LIST_VIRTUALIZE_MIN_ROWS,
  rowIndexForAnchor,
} from "./message-list-rows";
import type * as acp from "@agentclientprotocol/sdk";

interface MessageListProps {
  turns: ChatTurn[];
  /** Model switches to draw between the turns they happened after. */
  modelChanges?: ChatModelChange[];
  userName: string;
  isResponding: boolean;
  taskId?: string;
  projectId?: string;
  workspaceId?: string;
  availableCommands?: acp.AvailableCommand[];
  /** Optional presentation override for chats embedded inside another surface. */
  conversationNavigation?: ConversationNavigationPresentation;
}

const EMPTY_MODEL_CHANGES: ChatModelChange[] = [];

/** The scrollable turn thread, kept pinned to live ACP activity unless the reader scrolls away. */
export function MessageList({
  turns,
  modelChanges = EMPTY_MODEL_CHANGES,
  userName,
  isResponding,
  taskId,
  projectId,
  workspaceId,
  availableCommands = [],
  conversationNavigation,
}: MessageListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const workspaceQuery = useTaskWorkspace(taskId);
  const workspaceCwdQuery = useWorkspaceCwd(
    taskId === undefined ? workspaceId : undefined,
  );
  const cwd = workspaceQuery.data?.rootPath ?? workspaceCwdQuery.data ?? null;
  const parkedSelectionTextRef = useRef("");
  const [menuHasSelection, setMenuHasSelection] = useState(false);
  const [artifactCache] = useState(
    () => new Map<string, TurnArtifactCacheEntry>(),
  );
  const [viewportHeight, setViewportHeight] = useState(0);
  const artifactIndices = useMemo(
    () => collectCumulativeArtifactIndices(turns, artifactCache),
    [artifactCache, turns],
  );
  const artifactIndex = useMemo(
    () => artifactIndices.at(-1) ?? { edited: [], referenced: [] },
    [artifactIndices],
  );
  const chatLinkValue = useMemo(() => {
    if (taskId !== undefined) {
      return { index: artifactIndex, taskId, cwd };
    }
    if (projectId !== undefined) {
      return { index: artifactIndex, cwd };
    }
    return null;
  }, [artifactIndex, cwd, projectId, taskId]);
  const lastTurn = turns.at(-1);
  const lastAnchorId =
    lastTurn === undefined
      ? null
      : `${lastTurn.id}:${lastTurn.items.length === 0 && lastTurn.status === "streaming" ? "user" : "response"}`;
  const lastItem = lastTurn?.items.at(-1);
  const lastUserMessageId = lastTurn?.userMessage.id;
  // Hide the running indicator while the answer itself is streaming: the growing
  // text already shows the agent is live, so a second "working" line under it
  // just reads as noise. It returns for thoughts, tool calls, and the waits between.
  const streamingBody =
    lastItem?.kind === "message" && lastItem.role === "assistant";
  const showRunning = isResponding && !streamingBody;
  const rows = useMemo(
    () => buildMessageListRows(turns, modelChanges, showRunning),
    [modelChanges, showRunning, turns],
  );
  const windowed =
    viewportHeight > 0 && rows.length >= MESSAGE_LIST_VIRTUALIZE_MIN_ROWS;

  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (element === null) return;
    const sync = () => {
      const next = element.clientHeight;
      setViewportHeight((current) => (current === next ? current : next));
    };
    sync();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(sync);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  // eslint-disable-next-line react-hooks/incompatible-library -- the virtualizer instance is consumed during render (getTotalSize / getVirtualItems); compiler memoization of this list is not required
  const virtualizer = useVirtualizer({
    count: rows.length,
    enabled: windowed,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => estimateMessageListRowSize(rows[index]!),
    overscan: 6,
    getItemKey: (index) => rows[index]?.key ?? index,
    // Scroll restoration and follow-tail write scrollTop from layout effects.
    // A sync flush there trips React's "flushSync was called from inside a
    // lifecycle method" warning and the clean-stderr test gate.
    useFlushSync: false,
  });

  const ensureAnchorMounted = useCallback(
    (anchorId: string) => {
      if (!windowed) return;
      const index = rowIndexForAnchor(rows, turns, anchorId);
      if (index < 0) return;
      virtualizer.scrollToIndex(index, { align: "start" });
    },
    [rows, turns, virtualizer, windowed],
  );

  const navigation = useConversationNavigation({
    scrollRef,
    contentRef,
    followTailKey: `${turns.length}:${lastUserMessageId ?? ""}`,
    lastAnchorId,
    ensureAnchorMounted,
  });

  const chatLinkForTurn = useCallback(
    (turnIndex: number): ChatLinkContextValue | null => {
      const turnIndexValue = artifactIndices[turnIndex] ?? {
        edited: [],
        referenced: [],
      };
      if (taskId !== undefined) {
        return { index: turnIndexValue, taskId, cwd };
      }
      if (projectId !== undefined) {
        return { index: turnIndexValue, cwd };
      }
      return null;
    },
    [artifactIndices, cwd, projectId, taskId],
  );

  const renderedRows = windowed
    ? virtualizer.getVirtualItems().map((virtualItem) => {
        const row = rows[virtualItem.index];
        if (row === undefined) return null;
        return (
          <div
            key={row.key}
            data-index={virtualItem.index}
            ref={virtualizer.measureElement}
            className="absolute left-0 top-0 w-full"
            style={{ transform: `translateY(${virtualItem.start}px)` }}
          >
            <MessageListRowView
              row={row}
              turns={turns}
              userName={userName}
              availableCommands={availableCommands}
              chatLinkForTurn={chatLinkForTurn}
            />
          </div>
        );
      })
    : rows.map((row) => (
        <MessageListRowView
          key={row.key}
          row={row}
          turns={turns}
          userName={userName}
          availableCommands={availableCommands}
          chatLinkForTurn={chatLinkForTurn}
        />
      ));

  return (
    <ChatLinkContext.Provider value={chatLinkValue}>
      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden">
        <div className="min-h-0 flex-1 overflow-hidden">
          <TextEditContextMenu
            editable={false}
            hasSelection={menuHasSelection}
            trigger={
              <div
                ref={scrollRef}
                onScroll={navigation.handleScroll}
                onWheel={(event) => navigation.handleWheel(event.deltaY)}
                onPointerDown={navigation.beginPointerScroll}
                onPointerUp={navigation.endPointerScroll}
                onPointerCancel={navigation.endPointerScroll}
                onTouchStart={navigation.beginPointerScroll}
                onTouchEnd={navigation.endPointerScroll}
                onTouchCancel={navigation.endPointerScroll}
                data-testid="message-list"
                aria-live="polite"
                className="scrollbar-hide h-full min-h-0 flex-1 animate-in overflow-y-auto fade-in duration-500"
                onContextMenu={() => {
                  const text = window.getSelection()?.toString() ?? "";
                  parkedSelectionTextRef.current = text;
                  setMenuHasSelection(text.length > 0);
                }}
              />
            }
            onCut={() => undefined}
            onCopy={() => {
              const text = parkedSelectionTextRef.current;
              if (text.length === 0) {
                return;
              }
              void writeClipboardText(text);
            }}
            onPaste={() => undefined}
            onSelectAll={() => {
              const root = contentRef.current;
              if (root === null) {
                return;
              }
              selectElementContents(root);
              parkedSelectionTextRef.current =
                window.getSelection()?.toString() ?? "";
              setMenuHasSelection(parkedSelectionTextRef.current.length > 0);
            }}
          >
            <div
              ref={contentRef}
              className="mx-auto w-full max-w-[760px] px-3 pb-4 pt-5 sm:px-5 sm:pt-8"
              style={
                windowed
                  ? { height: virtualizer.getTotalSize(), position: "relative" }
                  : undefined
              }
            >
              {renderedRows}
            </div>
          </TextEditContextMenu>
        </div>
        <ConversationNavigator
          {...conversationNavigation}
          turns={turns}
          activeAnchorId={navigation.activeAnchorId}
          isAtTail={navigation.isAtTail}
          onNavigate={navigation.navigateToAnchor}
          onNavigateToTail={navigation.navigateToTail}
        />
      </div>
    </ChatLinkContext.Provider>
  );
}
