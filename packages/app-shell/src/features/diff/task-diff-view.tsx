import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Decoration,
  Diff,
  Hunk,
  getChangeKey,
  type ChangeData,
  type FileData,
  type GutterOptions,
} from "react-diff-view";
import "react-diff-view/style/index.css";
import "./task-diff-view.css";
import type {
  TaskDiffComment,
  TaskDiffCommentAnchor,
  TaskDiffScope,
  TaskDiffSide,
  TaskDiffThreadStatus,
} from "@ora/contracts";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Badge,
  Button,
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  type ResizablePanelHandle,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  Textarea,
} from "@ora/ui";
import {
  IconCheck,
  IconChevronDown,
  IconCode,
  IconFileDiff,
  IconGitBranch,
  IconMessageCircle,
  IconRefresh,
  IconUpload,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { useContractsClient } from "../../contracts-client-context";
import { localizeContractError } from "../../i18n/contract-error";
import { queryKeys } from "../../state/hooks/query-keys";
import {
  useTaskDiff,
  useTaskDiffComments,
} from "../../state/hooks/use-task-diff";
import {
  buildCollapsedDiffSegments,
  findNewSideLineTarget,
} from "./task-diff-collapse";
import { countChanges, parseTaskDiffPatch } from "./task-diff-data";
import { createCommentAnchor } from "./task-diff-comment-anchor";
import { diffFilePath } from "./task-diff-file-tree-utils";
import { TaskDiffFileTree } from "./task-diff-file-tree";
import { TaskGitActions } from "./task-git-actions";
import {
  animatePanelWidth,
  cancelPanelWidthAnimation,
} from "../../lib/panel-motion";
import { pathsMatchForWorkspace } from "../../lib/workspace-path";
import { diffFileScrollTop, isDiffFileScrollSettled } from "./task-diff-scroll";

/** Matches the review panel slide so the file tree toggle feels consistent. */
const FILE_TREE_SLIDE_MS = 180;
const FILE_TREE_WIDTH = 240;
/** Narrowest tree width a user resize settles on; below it the tree collapses. */
const FILE_TREE_MIN_WIDTH = 180;
const FILE_TREE_COLLAPSE_THRESHOLD = FILE_TREE_MIN_WIDTH / 2;

interface TaskDiffViewProps {
  taskId: string;
  viewType: TaskDiffViewType;
  fileTreeOpen: boolean;
  fileRequest?: TaskDiffFileRequest;
  toolbar?: ReactNode;
  onFileTreeOpenChange: (open: boolean) => void;
  onFileNotFound?: (path: string, line?: number) => void;
}

export type TaskDiffViewType = "unified" | "split";

export interface TaskDiffFileRequest {
  path: string;
  requestId: number;
  line?: number;
}

interface SelectedAnchor {
  anchor: TaskDiffCommentAnchor;
  changeKey: string;
}

/** Renders a task worktree patch and its line-anchored review discussions. */
export function TaskDiffView({
  taskId,
  viewType,
  fileTreeOpen,
  fileRequest,
  toolbar,
  onFileTreeOpenChange,
  onFileNotFound,
}: TaskDiffViewProps) {
  const { i18n, t } = useTranslation();
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const [scope, setScope] = useState<TaskDiffScope>("branch");
  const [gitActionsOpen, setGitActionsOpen] = useState(false);
  const [commitMessage, setCommitMessage] = useState("");
  const [pushOpen, setPushOpen] = useState(false);
  const [gitNotice, setGitNotice] = useState<string | null>(null);
  const diffQuery = useTaskDiff(taskId, scope);
  const commentsQuery = useTaskDiffComments(taskId);
  const [selectedAnchor, setSelectedAnchor] = useState<SelectedAnchor | null>(
    null,
  );
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [appliedFileRequestId, setAppliedFileRequestId] = useState<
    number | null
  >(null);
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const fileElementsRef = useRef(new Map<string, HTMLDivElement>());
  const fileTreePanelRef = useRef<ResizablePanelHandle | null>(null);
  const fileTreeAnimationRef = useRef<number | null>(null);
  const fileTreeWidthRef = useRef(FILE_TREE_WIDTH);
  // Programmatic jumps (chat links, tree clicks) must not be overwritten by the
  // scroll spy: on mount it treats an empty viewport as "scrolled to the end".
  const suppressScrollSyncRef = useRef(false);

  const files = useMemo(
    () =>
      diffQuery.data === undefined
        ? []
        : parseTaskDiffPatch(diffQuery.data.patch),
    [diffQuery.data],
  );
  const filePaths = useMemo(() => files.map(diffFilePath), [files]);
  const stats = useMemo(() => countChanges(files), [files]);
  const changedFilesLabel = t("diff.changedFilesLabel", {
    defaultValue:
      i18n.resolvedLanguage === "en-US" ? "changed files" : "个变更文件",
  });
  const activeFilePath =
    filePaths.length === 0
      ? ""
      : filePaths.some((path) => path === selectedFilePath)
        ? selectedFilePath!
        : filePaths[0]!;

  if (
    fileRequest !== undefined &&
    !diffQuery.isLoading &&
    fileRequest.requestId !== appliedFileRequestId
  ) {
    setAppliedFileRequestId(fileRequest.requestId);
    const matchingPath = filePaths.find((path) =>
      pathsMatchForWorkspace(fileRequest.path, path),
    );
    if (matchingPath !== undefined) {
      setSelectedAnchor(null);
      setSelectedFilePath(matchingPath);
    }
  }

  /**
   * Scrolls the Diff viewport so `path` sits near the top.
   * Returns false when the panel has not laid out yet so callers can retry
   * instead of treating a 0-height first paint as a completed jump.
   */
  const scrollToPath = useCallback((path: string, behavior: ScrollBehavior) => {
    const root = scrollContainerRef.current;
    const element = fileElementsRef.current.get(path);
    if (
      root === null ||
      element === undefined ||
      typeof root.scrollTo !== "function"
    ) {
      return false;
    }
    const top = diffFileScrollTop(root, element);
    if (top === null) return false;
    root.scrollTo({ top, behavior });
    return true;
  }, []);

  /** Selects a changed file and aligns its first line with the top of the Diff viewport. */
  const selectFile = useCallback(
    (path: string, behavior: ScrollBehavior = "smooth") => {
      suppressScrollSyncRef.current = true;
      setSelectedAnchor(null);
      setSelectedFilePath(path);
      if (scrollToPath(path, behavior)) return;
      requestAnimationFrame(() => {
        if (!scrollToPath(path, behavior))
          suppressScrollSyncRef.current = false;
      });
    },
    [scrollToPath],
  );

  useLayoutEffect(() => {
    if (fileRequest === undefined || diffQuery.isLoading) return;
    if (fileRequest.requestId !== appliedFileRequestId) return;
    const matchingPath = filePaths.find((path) =>
      pathsMatchForWorkspace(fileRequest.path, path),
    );
    if (matchingPath === undefined || matchingPath !== selectedFilePath) return;
    suppressScrollSyncRef.current = true;
    let cancelled = false;
    let succeeded = false;
    let attempts = 0;
    let observer: ResizeObserver | null = null;
    const attempt = () => {
      if (cancelled || succeeded) return;
      const root = scrollContainerRef.current;
      const element = fileElementsRef.current.get(matchingPath);
      scrollToPath(matchingPath, "auto");
      // Placeholder heights above the file can shrink after the first jump.
      // Keep retrying until the requested section is actually at the top.
      if (
        root !== null &&
        element !== undefined &&
        isDiffFileScrollSettled(root, element)
      ) {
        succeeded = true;
        observer?.disconnect();
        return;
      }
      if (++attempts < 90) {
        requestAnimationFrame(attempt);
        return;
      }
      suppressScrollSyncRef.current = false;
    };
    attempt();
    const root = scrollContainerRef.current;
    const target = fileElementsRef.current.get(matchingPath);
    const content = root?.firstElementChild;
    if (typeof ResizeObserver !== "undefined" && root !== null) {
      observer = new ResizeObserver(() => attempt());
      observer.observe(root);
      if (content instanceof Element) observer.observe(content);
      if (target !== undefined) observer.observe(target);
    }
    return () => {
      cancelled = true;
      observer?.disconnect();
    };
  }, [
    appliedFileRequestId,
    diffQuery.isLoading,
    filePaths,
    fileRequest,
    scrollToPath,
    selectedFilePath,
  ]);

  useEffect(() => {
    if (fileRequest === undefined || diffQuery.isLoading) return;
    if (fileRequest.requestId !== appliedFileRequestId) return;
    const matchingPath = filePaths.find((path) =>
      pathsMatchForWorkspace(fileRequest.path, path),
    );
    if (matchingPath === undefined) {
      onFileNotFound?.(fileRequest.path, fileRequest.line);
    }
  }, [
    fileRequest,
    appliedFileRequestId,
    filePaths,
    diffQuery.isLoading,
    onFileNotFound,
  ]);

  useEffect(() => {
    // The tree panel mounts collapsed alongside the diff, so toggling (or a late
    // mount once the patch arrives) slides it instead of snapping the diff width.
    cancelPanelWidthAnimation(fileTreeAnimationRef);
    animatePanelWidth({
      animationRef: fileTreeAnimationRef,
      duration: FILE_TREE_SLIDE_MS,
      panel: fileTreePanelRef.current,
      targetWidth: fileTreeOpen ? FILE_TREE_WIDTH : 0,
    });
  }, [fileTreeOpen, files.length]);

  // Never let a pending slide write to a panel that already left the tree.
  useEffect(() => () => cancelPanelWidthAnimation(fileTreeAnimationRef), []);

  /** Snaps an undersized tree after release so direct dragging stays linear. */
  const settleFileTreeAfterResize = useCallback(() => {
    const width = fileTreeWidthRef.current;
    if (width <= 0 || width >= FILE_TREE_MIN_WIDTH) return;
    cancelPanelWidthAnimation(fileTreeAnimationRef);
    animatePanelWidth({
      animationRef: fileTreeAnimationRef,
      duration: FILE_TREE_SLIDE_MS,
      panel: fileTreePanelRef.current,
      targetWidth:
        width < FILE_TREE_COLLAPSE_THRESHOLD ? 0 : FILE_TREE_MIN_WIDTH,
    });
  }, []);

  useEffect(() => {
    const root = scrollContainerRef.current;
    if (root === null || filePaths.length === 0) return;
    let frame: number | null = null;
    const updateActiveFile = () => {
      if (suppressScrollSyncRef.current) {
        suppressScrollSyncRef.current = false;
        return;
      }
      if (frame !== null) return;
      frame = requestAnimationFrame(() => {
        frame = null;
        let activePath = filePaths[0]!;
        if (root.scrollHeight - root.scrollTop - root.clientHeight <= 2) {
          activePath = filePaths.at(-1)!;
        } else {
          const rootTop = root.getBoundingClientRect().top;
          for (const path of filePaths) {
            const element = fileElementsRef.current.get(path);
            if (
              element === undefined ||
              element.getBoundingClientRect().top > rootTop + 48
            )
              break;
            activePath = path;
          }
        }
        setSelectedFilePath((currentPath) =>
          currentPath === activePath ? currentPath : activePath,
        );
      });
    };

    root.addEventListener("scroll", updateActiveFile, { passive: true });
    return () => {
      root.removeEventListener("scroll", updateActiveFile);
      if (frame !== null) cancelAnimationFrame(frame);
    };
  }, [filePaths]);

  const refreshDiscussions = () =>
    queryClient.invalidateQueries({
      queryKey: queryKeys.taskDiffComments(taskId),
    });

  const createComment = useMutation({
    mutationFn: ({
      scope,
      anchor,
      body,
    }: {
      scope: TaskDiffScope;
      anchor: TaskDiffCommentAnchor;
      body: string;
    }) => client.task.createDiffComment({ taskId, scope, anchor, body }),
    onSuccess: async () => {
      setSelectedAnchor(null);
      await refreshDiscussions();
    },
  });
  const replyComment = useMutation({
    mutationFn: ({ commentId, body }: { commentId: string; body: string }) =>
      client.task.replyDiffComment({ taskId, commentId, body }),
    onSuccess: refreshDiscussions,
  });
  const setCommentStatus = useMutation({
    mutationFn: ({
      commentId,
      status,
    }: {
      commentId: string;
      status: TaskDiffThreadStatus;
    }) => client.task.setDiffCommentStatus({ taskId, commentId, status }),
    onSuccess: refreshDiscussions,
  });
  const commitChanges = useMutation({
    mutationFn: (message: string) =>
      client.task.commitChanges({ taskId, message }),
    onSuccess: async (response) => {
      setGitActionsOpen(false);
      setCommitMessage("");
      setGitNotice(t("diff.commitSucceeded", { summary: response.summary }));
      setScope("committed");
      await queryClient.invalidateQueries({
        queryKey: queryKeys.taskDiffs(taskId),
      });
    },
  });
  const pushBranch = useMutation({
    mutationFn: () => client.task.pushBranch({ taskId }),
    onSuccess: (response) => {
      setPushOpen(false);
      setGitNotice(
        t("diff.pushSucceeded", {
          branch: response.branchName,
          remote: response.remoteName,
        }),
      );
    },
  });
  const commitAndPush = async () => {
    const message = commitMessage.trim();
    if (message === "") return;
    pushBranch.reset();
    setGitNotice(null);
    await commitChanges.mutateAsync(message);
    await pushBranch.mutateAsync();
  };
  const diff = diffQuery.data;
  const comments = useMemo(
    () => commentsQuery.data ?? [],
    [commentsQuery.data],
  );
  const diffId = diff?.diffId;
  const currentComments = useMemo(() => {
    if (scope !== "branch" || diffId === undefined) return [];
    return comments.filter(
      (comment) =>
        comment.kind.kind === "reply" || comment.kind.anchor.diffId === diffId,
    );
  }, [comments, diffId, scope]);
  const commentIndex = useMemo(
    () => buildDiffCommentIndex(currentComments),
    [currentComments],
  );
  const outdatedThreads = useMemo(
    () =>
      scope !== "branch" || diffId === undefined
        ? []
        : comments.filter(
            (comment) =>
              comment.kind.kind === "thread" &&
              comment.kind.anchor.diffId !== diffId,
          ),
    [comments, diffId, scope],
  );
  const handleSelectAnchor = useCallback(
    (selection: SelectedAnchor | null) => {
      createComment.reset();
      setSelectedAnchor(selection);
    },
    [createComment],
  );
  const handleCreateComment = useCallback(
    (anchor: TaskDiffCommentAnchor, body: string) =>
      createComment.mutateAsync({ scope, anchor, body }),
    [createComment, scope],
  );
  const handleReply = useCallback(
    (commentId: string, body: string) =>
      replyComment.mutateAsync({ commentId, body }),
    [replyComment],
  );
  const handleSetStatus = useCallback(
    (commentId: string, status: TaskDiffThreadStatus) =>
      setCommentStatus.mutateAsync({ commentId, status }),
    [setCommentStatus],
  );

  const gitActions = (
    <TaskGitActions
      open={gitActionsOpen}
      message={commitMessage}
      additions={stats.additions}
      deletions={stats.deletions}
      pending={commitChanges.isPending || pushBranch.isPending}
      onOpenChange={(open) => {
        if (open) {
          commitChanges.reset();
          pushBranch.reset();
          setGitNotice(null);
        }
        setGitActionsOpen(open);
      }}
      onMessageChange={setCommitMessage}
      onCommit={() => {
        setGitNotice(null);
        void commitChanges.mutateAsync(commitMessage.trim());
      }}
      onCommitAndPush={() => void commitAndPush()}
      onPush={() => {
        pushBranch.reset();
        setGitNotice(null);
        setGitActionsOpen(false);
        setPushOpen(true);
      }}
    />
  );

  const refresh = async () => {
    setSelectedAnchor(null);
    await Promise.all([diffQuery.refetch(), commentsQuery.refetch()]);
  };

  if (diffQuery.isLoading || commentsQuery.isLoading) {
    return <DiffLoadingState />;
  }

  if (diffQuery.error !== null || commentsQuery.error !== null) {
    const error = diffQuery.error ?? commentsQuery.error;
    return (
      <DiffMessage
        title={t("diff.loadError")}
        detail={localizeContractError(error, t)}
        action={
          <Button size="sm" variant="outline" onClick={() => void refresh()}>
            <IconRefresh />
            {t("diff.retry")}
          </Button>
        }
      />
    );
  }

  if (diff === undefined) return null;

  const mutationError =
    commitChanges.error ??
    pushBranch.error ??
    createComment.error ??
    replyComment.error ??
    setCommentStatus.error;

  return (
    <section
      className="relative flex h-full min-h-0 flex-1 flex-col overflow-hidden bg-background"
      aria-label={t("diff.taskChanges")}
      aria-busy={diffQuery.isFetching}
    >
      <header className="ora-diff-toolbar flex min-h-12 min-w-0 shrink-0 flex-nowrap items-center gap-2 overflow-hidden border-b border-border px-3 py-2 sm:px-4">
        <div className="ora-diff-toolbar__summary flex shrink-0 items-center gap-2 whitespace-nowrap">
          <span className="text-xs font-semibold">{files.length}</span>
          <span className="ora-diff-toolbar__summary-label text-xs font-semibold">
            {changedFilesLabel}
          </span>
          <span className="text-xs font-medium text-emerald-600">
            +{stats.additions}
          </span>
          <span className="text-xs font-medium text-red-600">
            −{stats.deletions}
          </span>
        </div>
        {gitActions}
        <div className="ora-diff-toolbar__scope-group flex h-8 shrink-0 items-center gap-0.5 rounded-lg bg-muted/50 p-0.5">
          <Select
            value={scope}
            onValueChange={(value) => {
              if (value === null) return;
              setSelectedAnchor(null);
              setScope(value as TaskDiffScope);
            }}
          >
            <SelectTrigger
              className="ora-diff-toolbar__scope-trigger h-7 w-20 gap-0.5 border-0 bg-transparent px-1 text-xs shadow-none hover:bg-background/70"
              aria-label={t("diff.scope")}
            >
              <IconGitBranch className="size-3.5 text-muted-foreground" />
              <span className="ora-diff-toolbar__scope-label min-w-0 flex-1 truncate text-left">
                {t(`diff.scope${scope[0]!.toUpperCase()}${scope.slice(1)}`)}
              </span>
            </SelectTrigger>
            <SelectContent align="start">
              <SelectItem value="branch">{t("diff.scopeBranch")}</SelectItem>
              <SelectItem value="unstaged">
                {t("diff.scopeUnstaged")}
              </SelectItem>
              <SelectItem value="staged">{t("diff.scopeStaged")}</SelectItem>
              <SelectItem value="committed">
                {t("diff.scopeCommitted")}
              </SelectItem>
            </SelectContent>
          </Select>
          <span className="h-4 w-px bg-border/70" aria-hidden="true" />
          <Button
            size="icon-sm"
            variant="ghost"
            className="ora-diff-toolbar__refresh size-7"
            aria-label={t("diff.refresh")}
            onClick={() => void refresh()}
          >
            <IconRefresh
              className={diffQuery.isFetching ? "animate-spin" : ""}
            />
          </Button>
        </div>
        <div className="flex-1" />
        <div className="ora-diff-toolbar__view-controls shrink-0">
          {toolbar}
        </div>
      </header>
      {diffQuery.isFetching && (
        <div
          role="status"
          aria-label={t("diff.refreshing")}
          className="pointer-events-none relative z-20 h-0 shrink-0"
        >
          <span className="ora-diff-progress absolute inset-x-0 top-0 block h-px w-1/3 bg-primary/70" />
        </div>
      )}
      {mutationError !== null && (
        <div
          role="alert"
          className="border-b border-destructive/20 bg-destructive/10 px-4 py-2 text-xs text-destructive"
        >
          {localizeContractError(mutationError, t)}
        </div>
      )}
      {gitNotice !== null && (
        <div
          role="status"
          className="border-b border-emerald-500/20 bg-emerald-500/10 px-4 py-2 text-xs text-emerald-700"
        >
          {gitNotice}
        </div>
      )}

      <div
        className={`flex min-h-0 flex-1 transition-opacity duration-150 ${
          diffQuery.isPlaceholderData ? "opacity-70" : "opacity-100"
        }`}
      >
        {files.length === 0 ? (
          <DiffMessage
            title={t("diff.noChanges")}
            detail={t("diff.noChangesDetail")}
          />
        ) : (
          <ResizablePanelGroup
            orientation="horizontal"
            onLayoutChanged={(_layout, meta) => {
              if (meta.isUserInteraction) settleFileTreeAfterResize();
            }}
          >
            <ResizablePanel
              id="task-diff-content"
              className="flex min-h-0 overflow-hidden"
              style={{ height: "100%", overflow: "hidden" }}
              minSize={280}
            >
              <div
                ref={scrollContainerRef}
                className="ora-scroll-region ora-diff-scroll-region h-full min-w-0 overflow-auto bg-background"
              >
                <div className="flex w-full flex-col pb-6 pl-4">
                  {files.map((file, fileIndex) => {
                    const path = diffFilePath(file);
                    return (
                      <div
                        key={`${file.oldPath}-${file.newPath}-${fileIndex}`}
                        ref={(element) => {
                          if (element === null)
                            fileElementsRef.current.delete(path);
                          else fileElementsRef.current.set(path, element);
                        }}
                        data-diff-path={path}
                        className="scroll-mt-0"
                      >
                        <TaskDiffFileViewport
                          file={file}
                          fileIndex={fileIndex}
                          viewType={viewType}
                          diffId={diff.diffId}
                          commentIndex={commentIndex}
                          reviewEnabled={scope === "branch"}
                          selectedAnchor={selectedAnchor}
                          onSelectAnchor={handleSelectAnchor}
                          onCreateComment={handleCreateComment}
                          onReply={handleReply}
                          onSetStatus={handleSetStatus}
                          mutationPending={
                            createComment.isPending ||
                            replyComment.isPending ||
                            setCommentStatus.isPending
                          }
                          targetLine={
                            fileRequest !== undefined &&
                            pathsMatchForWorkspace(fileRequest.path, path)
                              ? fileRequest.line
                              : undefined
                          }
                          rootRef={scrollContainerRef}
                          forceRender={activeFilePath === path}
                        />
                      </div>
                    );
                  })}

                  {outdatedThreads.length > 0 && (
                    <section className="rounded-lg border border-dashed border-border bg-background p-3">
                      <h3 className="text-xs font-semibold">
                        {t("diff.outdated")}
                      </h3>
                      <p className="mt-1 text-xs text-muted-foreground">
                        {t("diff.outdatedDetail")}
                      </p>
                      <div className="mt-3 space-y-2">
                        {outdatedThreads.map((comment) => (
                          <div
                            key={comment.id}
                            className="rounded-md bg-muted/50 px-3 py-2 text-xs"
                          >
                            <span className="font-mono text-muted-foreground">
                              {comment.kind.kind === "thread"
                                ? `${comment.kind.anchor.path}:${comment.kind.anchor.startLine}`
                                : ""}
                            </span>
                            <p className="mt-1 whitespace-pre-wrap">
                              {comment.body}
                            </p>
                          </div>
                        ))}
                      </div>
                    </section>
                  )}
                </div>
              </div>
            </ResizablePanel>
            <ResizableHandle
              withHandle
              aria-label={t("diff.resizeFileTree")}
              title={t("diff.resizeFileTree")}
              // Always visible so a collapsed tree can be dragged back open.
              className="z-10 transition-colors hover:bg-ring focus-visible:bg-ring"
              onPointerDown={() =>
                cancelPanelWidthAnimation(fileTreeAnimationRef)
              }
            />
            <ResizablePanel
              id="task-diff-files"
              panelRef={fileTreePanelRef}
              className="flex min-h-0 overflow-hidden"
              style={{ height: "100%", overflow: "hidden" }}
              defaultSize={fileTreeOpen ? FILE_TREE_WIDTH : 0}
              // A pixel min would snap scripted slides onto it; the settle
              // callback restores the effective minimum after the user lets go.
              minSize={1}
              maxSize={400}
              collapsible
              collapsedSize={0}
              groupResizeBehavior="preserve-pixel-size"
              onResize={(size) => {
                fileTreeWidthRef.current = size.inPixels;
                // Scripted slides (and lagging observer deliveries) report
                // transient sizes; only settled ones may flip the toolbar state,
                // or the toggle fights the slide and the tree won't reopen.
                if (fileTreeAnimationRef.current !== null) return;
                const open = size.inPixels > 0;
                if (open !== fileTreeOpen) onFileTreeOpenChange(open);
              }}
            >
              {fileTreeOpen && (
                <TaskDiffFileTree
                  files={files}
                  selectedPath={activeFilePath}
                  onSelect={(path) => selectFile(path)}
                />
              )}
            </ResizablePanel>
          </ResizablePanelGroup>
        )}
      </div>
      <PushBranchDialog
        open={pushOpen}
        pending={pushBranch.isPending}
        error={pushBranch.error}
        onOpenChange={setPushOpen}
        onPush={() => pushBranch.mutateAsync()}
      />
    </section>
  );
}

interface PushBranchDialogProps {
  open: boolean;
  pending: boolean;
  error: Error | null;
  onOpenChange: (open: boolean) => void;
  onPush: () => Promise<unknown>;
}

/** Confirms the network-visible push before publishing the task branch to origin. */
function PushBranchDialog({
  open,
  pending,
  error,
  onOpenChange,
  onPush,
}: PushBranchDialogProps) {
  const { t } = useTranslation();
  return (
    <AlertDialog
      open={open}
      onOpenChange={(nextOpen) => !pending && onOpenChange(nextOpen)}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("diff.pushDialogTitle")}</AlertDialogTitle>
          <AlertDialogDescription>
            {t("diff.pushDialogDescription")}
          </AlertDialogDescription>
        </AlertDialogHeader>
        {error !== null && (
          <p className="text-xs text-destructive">
            {localizeContractError(error, t)}
          </p>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={pending}>
            {t("common.cancel")}
          </AlertDialogCancel>
          <AlertDialogAction disabled={pending} onClick={() => void onPush()}>
            <IconUpload />
            {pending ? t("diff.pushing") : t("diff.push")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

interface TaskDiffFileProps {
  file: FileData;
  fileIndex: number;
  viewType: TaskDiffViewType;
  diffId: string;
  commentIndex: DiffCommentIndex;
  reviewEnabled: boolean;
  selectedAnchor: SelectedAnchor | null;
  onSelectAnchor: (selection: SelectedAnchor | null) => void;
  onCreateComment: (
    anchor: TaskDiffCommentAnchor,
    body: string,
  ) => Promise<unknown>;
  onReply: (commentId: string, body: string) => Promise<unknown>;
  onSetStatus: (
    commentId: string,
    status: TaskDiffThreadStatus,
  ) => Promise<unknown>;
  mutationPending: boolean;
  targetLine?: number;
}

interface DiffCommentIndex {
  threadsByAnchor: Map<string, TaskDiffComment[]>;
  repliesByParent: Map<string, TaskDiffComment[]>;
}

/** Builds lookup tables so each diff line does not scan every review comment. */
function buildDiffCommentIndex(comments: TaskDiffComment[]): DiffCommentIndex {
  const threadsByAnchor = new Map<string, TaskDiffComment[]>();
  const repliesByParent = new Map<string, TaskDiffComment[]>();
  for (const comment of comments) {
    if (comment.kind.kind === "reply") {
      const replies = repliesByParent.get(comment.kind.parentCommentId) ?? [];
      replies.push(comment);
      repliesByParent.set(comment.kind.parentCommentId, replies);
      continue;
    }

    const key = `${comment.kind.anchor.path}:${comment.kind.anchor.startLine}`;
    const threads = threadsByAnchor.get(key) ?? [];
    threads.push(comment);
    threadsByAnchor.set(key, threads);
  }
  return { threadsByAnchor, repliesByParent };
}

/** Renders one parsed patch file and injects review widgets below matching lines. */
function TaskDiffFile({
  file,
  fileIndex,
  viewType,
  diffId,
  commentIndex,
  reviewEnabled,
  selectedAnchor,
  onSelectAnchor,
  onCreateComment,
  onReply,
  onSetStatus,
  mutationPending,
  targetLine,
}: TaskDiffFileProps) {
  const { t } = useTranslation();
  const fileRootRef = useRef<HTMLElement | null>(null);
  const [expanded, setExpanded] = useState(true);
  const [expandedBlocks, setExpandedBlocks] = useState<Set<string>>(
    () => new Set(),
  );
  const fileStats = useMemo(() => countChanges([file]), [file]);
  const jumpTarget =
    targetLine === undefined
      ? null
      : findNewSideLineTarget(file.hunks, targetLine);
  const jumpChangeKey =
    jumpTarget === null ? null : getChangeKey(jumpTarget.change);
  if (targetLine !== undefined && !expanded) {
    setExpanded(true);
  }
  if (
    jumpTarget !== null &&
    jumpTarget.collapsedKey !== null &&
    !expandedBlocks.has(jumpTarget.collapsedKey)
  ) {
    const next = new Set(expandedBlocks);
    next.add(jumpTarget.collapsedKey);
    setExpandedBlocks(next);
  }
  const renderSegments = useMemo(
    () => buildCollapsedDiffSegments(file.hunks, expandedBlocks),
    [expandedBlocks, file.hunks],
  );
  const selectedChangeKey = selectedAnchor?.changeKey.startsWith(
    `${fileIndex}:`,
  )
    ? selectedAnchor.changeKey.slice(`${fileIndex}:`.length)
    : null;

  useLayoutEffect(() => {
    if (jumpChangeKey === null) return;
    const selected = fileRootRef.current?.querySelector(
      ".diff-code-selected, .diff-selected",
    );
    selected?.scrollIntoView({ block: "center", inline: "nearest" });
  }, [jumpChangeKey, renderSegments]);

  const widgets = useMemo(
    () =>
      Object.fromEntries(
        file.hunks.flatMap((hunk) =>
          hunk.changes.flatMap((change) => {
            const changeKey = getChangeKey(change);
            const oldLine = lineNumberFor(change, "old");
            const newLine = lineNumberFor(change, "new");
            const oldThreads =
              oldLine === null
                ? []
                : (commentIndex.threadsByAnchor.get(
                    `${file.oldPath}:${oldLine}`,
                  ) ?? []);
            const newThreads =
              newLine === null
                ? []
                : (commentIndex.threadsByAnchor.get(
                    `${file.newPath}:${newLine}`,
                  ) ?? []);
            const matchingThreads =
              file.oldPath === file.newPath && oldLine === newLine
                ? oldThreads
                : [...oldThreads, ...newThreads];
            const isSelected = selectedChangeKey === changeKey;
            if (matchingThreads.length === 0 && !isSelected) return [];

            return [
              [
                changeKey,
                <div className="space-y-2">
                  {matchingThreads.map((thread) => (
                    <DiffThread
                      key={thread.id}
                      thread={thread}
                      replies={
                        commentIndex.repliesByParent.get(thread.id) ?? []
                      }
                      onReply={onReply}
                      onSetStatus={onSetStatus}
                      disabled={mutationPending}
                    />
                  ))}
                  {isSelected && selectedAnchor !== null && (
                    <CommentComposer
                      anchor={selectedAnchor.anchor}
                      onCancel={() => onSelectAnchor(null)}
                      onSubmit={(body) =>
                        onCreateComment(selectedAnchor.anchor, body)
                      }
                      disabled={mutationPending}
                    />
                  )}
                </div>,
              ] as const,
            ];
          }),
        ),
      ),
    [
      commentIndex,
      file,
      mutationPending,
      onCreateComment,
      onReply,
      onSelectAnchor,
      onSetStatus,
      selectedAnchor,
      selectedChangeKey,
    ],
  );

  const selectLine = useCallback(
    ({ change, side }: { change: ChangeData | null; side?: TaskDiffSide }) => {
      if (!reviewEnabled || change === null) return;
      const resolvedSide = resolveSide(change, side);
      const lineNumber = lineNumberFor(change, resolvedSide);
      if (lineNumber === null) return;
      const hunk = file.hunks.find((candidate) =>
        candidate.changes.includes(change),
      );
      if (hunk === undefined) return;

      onSelectAnchor({
        changeKey: `${fileIndex}:${getChangeKey(change)}`,
        anchor: createCommentAnchor(file, hunk, change, resolvedSide, diffId),
      });
    },
    [diffId, file, fileIndex, onSelectAnchor, reviewEnabled],
  );
  const gutterEvents = useMemo(() => ({ onClick: selectLine }), [selectLine]);

  return (
    <article ref={fileRootRef} className="bg-background">
      <header className="sticky top-0 z-10 border-b border-border/60 bg-background/95 backdrop-blur">
        <button
          type="button"
          className="flex min-h-10 w-full items-center gap-2 px-2 py-2 text-left outline-none transition-colors hover:bg-muted/35 focus-visible:bg-muted/35 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
          aria-expanded={expanded}
          aria-label={t(expanded ? "diff.collapseFile" : "diff.expandFile", {
            path: displayPath(file),
          })}
          onClick={() => setExpanded((current) => !current)}
        >
          <IconChevronDown
            className={`size-3.5 shrink-0 text-muted-foreground transition-transform ${expanded ? "" : "-rotate-90"}`}
            aria-hidden="true"
          />
          <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-violet-500/12 text-violet-700 ring-1 ring-inset ring-violet-500/15 dark:text-violet-300">
            <IconFileDiff className="size-3.5" />
          </span>
          <span
            className="min-w-0 flex-1 truncate font-mono text-xs"
            title={displayPath(file)}
          >
            {displayPath(file)}
          </span>
          <span className="shrink-0 text-xs tabular-nums text-emerald-600">
            +{fileStats.additions}
          </span>
          <span className="shrink-0 text-xs tabular-nums text-red-600">
            −{fileStats.deletions}
          </span>
        </button>
      </header>
      {expanded &&
        (file.hunks.length === 0 ? (
          <div className="px-4 py-8 text-center text-xs text-muted-foreground">
            {file.isBinary ? t("diff.binary") : t("diff.metadataOnly")}
          </div>
        ) : (
          <div
            className={`ora-task-diff ora-task-diff--${viewType} ora-task-diff--${file.type} ${
              reviewEnabled ? "ora-task-diff--reviewable" : ""
            } overflow-x-auto`}
          >
            {viewType === "split" && (
              <div className="ora-diff-version-headings" aria-hidden="true">
                <span>{t("diff.modifiedFile")}</span>
                <span>{t("diff.originalFile")}</span>
              </div>
            )}
            <Diff
              viewType={viewType}
              diffType={file.type}
              hunks={file.hunks}
              widgets={widgets}
              selectedChanges={[
                ...(selectedChangeKey === null ? [] : [selectedChangeKey]),
                ...(jumpChangeKey === null ? [] : [jumpChangeKey]),
              ]}
              gutterEvents={gutterEvents}
              renderGutter={
                viewType === "unified" ? renderSingleLineNumber : undefined
              }
              optimizeSelection
            >
              {() =>
                renderSegments.map((segment) =>
                  segment.kind === "hunk" ? (
                    <Hunk key={segment.key} hunk={segment.hunk} />
                  ) : (
                    <Decoration
                      key={segment.key}
                      className="ora-diff-collapsed"
                      contentClassName="ora-diff-collapsed-cell"
                    >
                      <button
                        type="button"
                        className="group flex h-8 w-full items-center justify-center gap-2 text-[11px] text-muted-foreground outline-none transition-colors hover:bg-violet-500/8 hover:text-foreground focus-visible:bg-violet-500/10 focus-visible:text-foreground"
                        aria-label={t("diff.expandUnchanged", {
                          count: segment.lineCount,
                        })}
                        onClick={() => {
                          setExpandedBlocks((current) => {
                            const next = new Set(current);
                            next.add(segment.key);
                            return next;
                          });
                        }}
                      >
                        <span className="flex size-5 items-center justify-center rounded-md bg-violet-500/10 text-violet-700 transition-colors group-hover:bg-violet-500/15 dark:text-violet-300">
                          <IconChevronDown className="size-3.5" />
                        </span>
                        {t("diff.unchangedLinesHidden", {
                          count: segment.lineCount,
                        })}
                      </button>
                    </Decoration>
                  ),
                )
              }
            </Diff>
          </div>
        ))}
    </article>
  );
}

/** Compares one file's inputs while ignoring selection changes that belong to other files. */
function areTaskDiffFilePropsEqual(
  previous: TaskDiffFileProps,
  next: TaskDiffFileProps,
): boolean {
  const previousSelection = previous.selectedAnchor?.changeKey.startsWith(
    `${previous.fileIndex}:`,
  )
    ? previous.selectedAnchor
    : null;
  const nextSelection = next.selectedAnchor?.changeKey.startsWith(
    `${next.fileIndex}:`,
  )
    ? next.selectedAnchor
    : null;
  return (
    previous.file === next.file &&
    previous.fileIndex === next.fileIndex &&
    previous.viewType === next.viewType &&
    previous.diffId === next.diffId &&
    previous.commentIndex === next.commentIndex &&
    previous.reviewEnabled === next.reviewEnabled &&
    previous.onSelectAnchor === next.onSelectAnchor &&
    previous.onCreateComment === next.onCreateComment &&
    previous.onReply === next.onReply &&
    previous.onSetStatus === next.onSetStatus &&
    previous.mutationPending === next.mutationPending &&
    previous.targetLine === next.targetLine &&
    previousSelection === nextSelection
  );
}

const MemoizedTaskDiffFile = memo(TaskDiffFile, areTaskDiffFilePropsEqual);

interface TaskDiffFileViewportProps extends TaskDiffFileProps {
  rootRef: RefObject<HTMLDivElement | null>;
  forceRender: boolean;
}

/** Mounts nearby diff files on demand so large patches do not create one large DOM tree at once. */
function TaskDiffFileViewport({
  rootRef,
  forceRender,
  file,
  ...fileProps
}: TaskDiffFileViewportProps) {
  const elementRef = useRef<HTMLDivElement | null>(null);
  const supportsIntersectionObserver =
    typeof IntersectionObserver !== "undefined";
  const [isNearViewport, setIsNearViewport] = useState(
    () => forceRender || !supportsIntersectionObserver,
  );
  const shouldRender = forceRender || isNearViewport;
  const estimatedHeight = useMemo(
    () =>
      Math.max(
        72,
        48 +
          file.hunks.reduce((total, hunk) => total + hunk.changes.length, 0) *
            24,
      ),
    [file.hunks],
  );

  useEffect(() => {
    if (shouldRender) return;

    const element = elementRef.current;
    const root = rootRef.current;
    if (element === null || root === null) {
      const frame = requestAnimationFrame(() => setIsNearViewport(true));
      return () => cancelAnimationFrame(frame);
    }

    const observer = new IntersectionObserver(
      ([entry]) => {
        if (!entry?.isIntersecting) return;
        setIsNearViewport(true);
        observer.disconnect();
      },
      { root, rootMargin: "1200px 0px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, [rootRef, shouldRender]);

  return (
    <div
      ref={elementRef}
      className="ora-diff-file-viewport"
      style={shouldRender ? undefined : { minHeight: estimatedHeight }}
      aria-busy={!shouldRender}
    >
      {shouldRender ? (
        <MemoizedTaskDiffFile file={file} {...fileProps} />
      ) : null}
    </div>
  );
}

interface DiffThreadProps {
  thread: TaskDiffComment;
  replies: TaskDiffComment[];
  onReply: (commentId: string, body: string) => Promise<unknown>;
  onSetStatus: (
    commentId: string,
    status: TaskDiffThreadStatus,
  ) => Promise<unknown>;
  disabled: boolean;
}

/** Displays one root review discussion, its replies, and lifecycle controls. */
function DiffThread({
  thread,
  replies,
  onReply,
  onSetStatus,
  disabled,
}: DiffThreadProps) {
  const { t } = useTranslation();
  const [reply, setReply] = useState("");
  if (thread.kind.kind !== "thread") return null;
  const nextStatus = thread.kind.status === "open" ? "resolved" : "open";

  const submitReply = async () => {
    const body = reply.trim();
    if (body === "") return;
    await onReply(thread.id, body);
    setReply("");
  };

  return (
    <section className="rounded-md border border-border bg-background text-xs shadow-sm">
      <header className="flex items-center gap-2 border-b border-border px-3 py-2">
        <IconMessageCircle className="size-3.5 text-muted-foreground" />
        <span className="font-medium">{t("diff.discussion")}</span>
        <Badge
          variant={thread.kind.status === "open" ? "secondary" : "outline"}
          className="text-[10px]"
        >
          {thread.kind.status}
        </Badge>
        <div className="flex-1" />
        <Button
          size="xs"
          variant="ghost"
          disabled={disabled}
          onClick={() => void onSetStatus(thread.id, nextStatus)}
        >
          <IconCheck />
          {nextStatus === "resolved" ? t("diff.resolve") : t("diff.reopen")}
        </Button>
      </header>
      <div className="space-y-2 px-3 py-2">
        <p className="whitespace-pre-wrap leading-5">{thread.body}</p>
        {replies.map((replyMessage) => (
          <div key={replyMessage.id} className="border-l-2 border-border pl-3">
            <p className="whitespace-pre-wrap leading-5">{replyMessage.body}</p>
          </div>
        ))}
        <div className="flex items-end gap-2">
          <Textarea
            value={reply}
            onChange={(event) => setReply(event.target.value)}
            rows={1}
            placeholder={t("diff.replyPlaceholder")}
            aria-label={t("diff.reply")}
            className="min-h-8 resize-y text-xs"
          />
          <Button
            size="sm"
            disabled={disabled || reply.trim() === ""}
            onClick={() => void submitReply()}
          >
            {t("diff.reply")}
          </Button>
        </div>
      </div>
    </section>
  );
}

interface CommentComposerProps {
  anchor: TaskDiffCommentAnchor;
  onCancel: () => void;
  onSubmit: (body: string) => Promise<unknown>;
  disabled: boolean;
}

/** Collects a new root discussion for the currently selected diff line. */
function CommentComposer({
  anchor,
  onCancel,
  onSubmit,
  disabled,
}: CommentComposerProps) {
  const { t } = useTranslation();
  const [body, setBody] = useState("");
  const composerRef = useRef<HTMLElement | null>(null);

  useEffect(() => {
    // Wide patches can place the widget off-screen horizontally, so reveal its full action area.
    composerRef.current?.scrollIntoView?.({
      block: "nearest",
      inline: "nearest",
    });
  }, []);

  const submit = async () => {
    const comment = body.trim();
    if (comment === "") return;
    await onSubmit(comment);
  };

  return (
    <section
      ref={composerRef}
      className="ora-comment-composer flex max-h-[min(20rem,calc(100vh-9rem))] flex-col rounded-md border border-primary/30 bg-background p-3 text-xs shadow-sm"
    >
      <p className="mb-2 font-medium">
        {t("diff.commentOn", { path: anchor.path, line: anchor.startLine })}
      </p>
      <Textarea
        autoFocus
        value={body}
        onChange={(event) => setBody(event.target.value)}
        rows={3}
        placeholder={t("diff.commentPlaceholder")}
        aria-label={t("diff.commentLabel")}
        className="min-h-16 max-h-40 resize-none overflow-y-auto text-xs"
      />
      <div className="mt-2 flex shrink-0 justify-end gap-2">
        <Button
          size="sm"
          variant="ghost"
          disabled={disabled}
          onClick={onCancel}
        >
          {t("common.cancel")}
        </Button>
        <Button
          size="sm"
          disabled={disabled || body.trim() === ""}
          onClick={() => void submit()}
        >
          {t("diff.addComment")}
        </Button>
      </div>
    </section>
  );
}

interface DiffMessageProps {
  title: string;
  detail: string;
  action?: React.ReactNode;
}

/** Keeps the Changes layout stable while its first snapshot is being loaded. */
function DiffLoadingState() {
  const { t } = useTranslation();
  return (
    <section
      className="flex h-full min-h-0 flex-1 flex-col overflow-hidden bg-background"
      aria-label={t("diff.taskChanges")}
      aria-busy="true"
    >
      <span role="status" className="sr-only">
        {t("diff.loading")}
      </span>
      <header className="flex h-12 shrink-0 animate-pulse items-center gap-3 border-b border-border py-2 pl-4 pr-40">
        <span className="h-3 w-28 rounded-full bg-muted" />
        <span className="h-7 w-24 rounded-md bg-muted/80" />
        <span className="flex-1" />
        <span className="h-7 w-16 rounded-md bg-muted/70" />
        <span className="h-7 w-16 rounded-md bg-muted/70" />
      </header>
      <div className="flex min-h-0 flex-1 animate-pulse">
        <div className="min-w-0 flex-1 space-y-5 overflow-hidden px-4 py-3">
          {[0, 1, 2].map((index) => (
            <div key={index} className="space-y-2">
              <div className="h-7 rounded-md bg-muted/65" />
              <div className="space-y-1">
                <div className="h-5 rounded-sm bg-muted/35" />
                <div className="h-5 w-11/12 rounded-sm bg-muted/35" />
                <div className="h-5 w-4/5 rounded-sm bg-muted/35" />
              </div>
            </div>
          ))}
        </div>
        <aside className="w-60 shrink-0 space-y-3 border-l border-border px-3 py-3">
          <div className="h-3 w-16 rounded-full bg-muted" />
          <div className="h-6 w-4/5 rounded-sm bg-muted/55" />
          <div className="h-6 w-3/5 rounded-sm bg-muted/55" />
          <div className="h-6 w-11/12 rounded-sm bg-muted/55" />
        </aside>
      </div>
    </section>
  );
}

/** Shows a centered task-diff loading, empty, or error state. */
function DiffMessage({ title, detail, action }: DiffMessageProps) {
  return (
    <div className="flex min-h-0 flex-1 items-center justify-center p-8">
      <div className="max-w-sm text-center">
        <IconCode className="mx-auto size-6 text-muted-foreground" />
        <h2 className="mt-3 text-sm font-semibold">{title}</h2>
        <p className="mt-1 text-xs leading-5 text-muted-foreground">{detail}</p>
        {action && <div className="mt-4">{action}</div>}
      </div>
    </div>
  );
}

/** Shows one current line number for context rows while retaining old numbers for deletions. */
function renderSingleLineNumber({
  change,
  side,
  renderDefault,
  wrapInAnchor,
}: GutterOptions) {
  if (change.type === "normal" && side === "old") return null;
  return wrapInAnchor(renderDefault());
}

/** Chooses the source side represented by a clicked diff cell. */
function resolveSide(change: ChangeData, side?: TaskDiffSide): TaskDiffSide {
  if (change.type === "delete") return "old";
  if (change.type === "insert") return "new";
  return side ?? "new";
}

/** Returns the old or new source line represented by one parsed change. */
function lineNumberFor(change: ChangeData, side: TaskDiffSide): number | null {
  if (change.type === "normal") {
    return side === "old" ? change.oldLineNumber : change.newLineNumber;
  }
  if (change.type === "delete")
    return side === "old" ? change.lineNumber : null;
  return side === "new" ? change.lineNumber : null;
}

/** Chooses the path users expect for added, deleted, and renamed files. */
function displayPath(file: FileData): string {
  return file.type === "delete" ? file.oldPath : file.newPath;
}
