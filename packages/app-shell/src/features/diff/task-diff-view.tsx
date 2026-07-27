import { useEffect, useMemo, useRef, useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Decoration,
  Diff,
  Hunk,
  getChangeKey,
  parseDiff,
  type ChangeData,
  type FileData,
  type GutterOptions,
  type HunkData,
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
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
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
  IconGitCommit,
  IconMessageCircle,
  IconRefresh,
  IconUpload,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "../../state/hooks/query-keys";
import { useTaskDiff, useTaskDiffComments } from "../../state/hooks/use-task-diff";
import { buildCollapsedDiffSegments } from "./task-diff-collapse";
import { diffFilePath, TaskDiffFileTree } from "./task-diff-file-tree";

interface TaskDiffViewProps {
  taskId: string;
  viewType: TaskDiffViewType;
  fileTreeOpen: boolean;
  expanded?: boolean;
  onFileTreeOpenChange: (open: boolean) => void;
}

export type TaskDiffViewType = "unified" | "split";

interface SelectedAnchor {
  anchor: TaskDiffCommentAnchor;
  changeKey: string;
}

interface DiffStats {
  additions: number;
  deletions: number;
}

/** Renders a task worktree patch and its line-anchored review discussions. */
export function TaskDiffView({
  taskId,
  viewType,
  fileTreeOpen,
  expanded = false,
  onFileTreeOpenChange,
}: TaskDiffViewProps) {
  const { t } = useTranslation();
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const [scope, setScope] = useState<TaskDiffScope>("branch");
  const [commitOpen, setCommitOpen] = useState(false);
  const [commitMessage, setCommitMessage] = useState("");
  const [pushOpen, setPushOpen] = useState(false);
  const [gitNotice, setGitNotice] = useState<string | null>(null);
  const diffQuery = useTaskDiff(taskId, scope);
  const commentsQuery = useTaskDiffComments(taskId);
  const [selectedAnchor, setSelectedAnchor] = useState<SelectedAnchor | null>(null);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const fileElementsRef = useRef(new Map<string, HTMLDivElement>());
  const fileTreePanelRef = useRef<ResizablePanelHandle | null>(null);

  const files = useMemo(
    () => diffQuery.data === undefined ? [] : parseDiff(diffQuery.data.patch),
    [diffQuery.data],
  );
  const stats = useMemo(() => countChanges(files), [files]);
  const activeFilePath = files.length === 0
    ? ""
    : files.some((file) => diffFilePath(file) === selectedFilePath)
      ? selectedFilePath!
      : diffFilePath(files[0]!);

  useEffect(() => {
    const panel = fileTreePanelRef.current;
    if (panel === null) return;
    if (fileTreeOpen && panel.isCollapsed()) panel.expand();
    if (!fileTreeOpen && !panel.isCollapsed()) panel.collapse();
  }, [fileTreeOpen, files.length]);

  useEffect(() => {
    const root = scrollContainerRef.current;
    if (root === null || files.length === 0) return;
    const updateActiveFile = () => {
      const paths = files.map(diffFilePath);
      if (root.scrollHeight - root.scrollTop - root.clientHeight <= 2) {
        setSelectedFilePath(paths.at(-1)!);
        return;
      }

      const rootTop = root.getBoundingClientRect().top;
      let activePath = paths[0]!;
      for (const path of paths) {
        const element = fileElementsRef.current.get(path);
        if (element === undefined || element.getBoundingClientRect().top > rootTop + 48) break;
        activePath = path;
      }
      setSelectedFilePath(activePath);
    };

    updateActiveFile();
    root.addEventListener("scroll", updateActiveFile, { passive: true });
    return () => root.removeEventListener("scroll", updateActiveFile);
  }, [files]);

  const refreshDiscussions = () =>
    queryClient.invalidateQueries({ queryKey: queryKeys.taskDiffComments(taskId) });

  const createComment = useMutation({
    mutationFn: ({ anchor, body }: { anchor: TaskDiffCommentAnchor; body: string }) =>
      client.task.createDiffComment({ taskId, anchor, body }),
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
    mutationFn: ({ commentId, status }: { commentId: string; status: TaskDiffThreadStatus }) =>
      client.task.setDiffCommentStatus({ taskId, commentId, status }),
    onSuccess: refreshDiscussions,
  });
  const commitChanges = useMutation({
    mutationFn: (message: string) => client.task.commitChanges({ taskId, message }),
    onSuccess: async (response) => {
      setCommitOpen(false);
      setCommitMessage("");
      setGitNotice(t("diff.commitSucceeded", { summary: response.summary }));
      setScope("committed");
      await queryClient.invalidateQueries({ queryKey: queryKeys.taskDiffs(taskId) });
    },
  });
  const pushBranch = useMutation({
    mutationFn: () => client.task.pushBranch({ taskId }),
    onSuccess: (response) => {
      setPushOpen(false);
      setGitNotice(t("diff.pushSucceeded", {
        branch: response.branchName,
        remote: response.remoteName,
      }));
    },
  });

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
        detail={error instanceof Error ? error.message : t("diff.requestFailed")}
        action={<Button size="sm" variant="outline" onClick={() => void refresh()}><IconRefresh />{t("diff.retry")}</Button>}
      />
    );
  }

  const diff = diffQuery.data;
  if (diff === undefined) return null;

  const comments = commentsQuery.data ?? [];
  const mutationError = commitChanges.error
    ?? pushBranch.error
    ?? createComment.error
    ?? replyComment.error
    ?? setCommentStatus.error;
  const currentComments = scope === "branch" ? comments.filter(
    (comment) => comment.kind.kind === "reply"
      || comment.kind.anchor.diffId === diff.diffId,
  ) : [];
  const outdatedThreads = scope === "branch" ? comments.filter(
    (comment) => comment.kind.kind === "thread"
      && comment.kind.anchor.diffId !== diff.diffId,
  ) : [];

  return (
    <section
      className="relative flex h-full min-h-0 flex-1 flex-col overflow-hidden bg-background"
      aria-label={t("diff.taskChanges")}
      aria-busy={diffQuery.isFetching}
    >
      <header
        className={`flex min-h-12 shrink-0 flex-wrap items-center gap-2 border-b border-border py-2 pl-3 sm:pl-4 ${
          expanded ? "pr-[13.5rem]" : "pr-40"
        }`}
      >
        <div className="flex min-w-0 items-center gap-2">
          <IconCode className="size-4 text-muted-foreground" />
          <span className="text-xs font-semibold">{t("diff.changedFiles", { count: files.length })}</span>
          <span className="text-xs font-medium text-emerald-600">+{stats.additions}</span>
          <span className="text-xs font-medium text-red-600">−{stats.deletions}</span>
        </div>
        <div className="flex h-8 items-center gap-0.5 rounded-lg bg-muted/50 p-0.5">
          <Select
            value={scope}
            onValueChange={(value) => {
              if (value === null) return;
              setSelectedAnchor(null);
              setScope(value as TaskDiffScope);
            }}
          >
            <SelectTrigger
              className="h-7 w-28 border-0 bg-transparent px-2 text-xs shadow-none hover:bg-background/70"
              aria-label={t("diff.scope")}
            >
              <IconGitBranch className="size-3.5 text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate text-left">
                {t(`diff.scope${scope[0]!.toUpperCase()}${scope.slice(1)}`)}
              </span>
            </SelectTrigger>
            <SelectContent align="start">
              <SelectItem value="branch">{t("diff.scopeBranch")}</SelectItem>
              <SelectItem value="unstaged">{t("diff.scopeUnstaged")}</SelectItem>
              <SelectItem value="staged">{t("diff.scopeStaged")}</SelectItem>
              <SelectItem value="committed">{t("diff.scopeCommitted")}</SelectItem>
            </SelectContent>
          </Select>
          <span className="h-4 w-px bg-border/70" aria-hidden="true" />
          <Button
            size="icon-sm"
            variant="ghost"
            className="size-7"
            aria-label={t("diff.refresh")}
            onClick={() => void refresh()}
          >
            <IconRefresh className={diffQuery.isFetching ? "animate-spin" : ""} />
          </Button>
        </div>
        <div className="flex-1" />
        <div className="flex h-8 items-center gap-0.5 rounded-lg border border-border/70 bg-background p-0.5 shadow-xs">
          <Button
            size="sm"
            variant="ghost"
            className="h-7 px-2.5"
            disabled={commitChanges.isPending || pushBranch.isPending}
            onClick={() => {
              commitChanges.reset();
              setGitNotice(null);
              setCommitOpen(true);
            }}
          >
            <IconGitCommit />{t("diff.commit")}
          </Button>
          <span className="h-4 w-px bg-border/70" aria-hidden="true" />
          <Button
            size="sm"
            variant="ghost"
            className="h-7 px-2.5"
            disabled={commitChanges.isPending || pushBranch.isPending}
            onClick={() => {
              pushBranch.reset();
              setGitNotice(null);
              setPushOpen(true);
            }}
          >
            <IconUpload />{t("diff.push")}
          </Button>
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
        <div role="alert" className="border-b border-destructive/20 bg-destructive/10 px-4 py-2 text-xs text-destructive">
          {mutationError instanceof Error ? mutationError.message : t("diff.reviewFailed")}
        </div>
      )}
      {gitNotice !== null && (
        <div role="status" className="border-b border-emerald-500/20 bg-emerald-500/10 px-4 py-2 text-xs text-emerald-700">
          {gitNotice}
        </div>
      )}

      <div
        className={`flex min-h-0 flex-1 transition-opacity duration-150 ${
          diffQuery.isPlaceholderData ? "opacity-70" : "opacity-100"
        }`}
      >
        {files.length === 0 ? (
          <DiffMessage title={t("diff.noChanges")} detail={t("diff.noChangesDetail")} />
        ) : (
          <ResizablePanelGroup orientation="horizontal">
            <ResizablePanel
              id="task-diff-content"
              className="flex min-h-0 overflow-hidden"
              style={{ height: "100%", overflow: "hidden" }}
              minSize={320}
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
                          if (element === null) fileElementsRef.current.delete(path);
                          else fileElementsRef.current.set(path, element);
                        }}
                        data-diff-path={path}
                        className="scroll-mt-0"
                      >
                        <TaskDiffFile
                          file={file}
                          fileIndex={fileIndex}
                          viewType={viewType}
                          diffId={diff.diffId}
                          comments={currentComments}
                          reviewEnabled={scope === "branch"}
                          selectedAnchor={selectedAnchor}
                          onSelectAnchor={(selection) => {
                            createComment.reset();
                            setSelectedAnchor(selection);
                          }}
                          onCreateComment={(anchor, body) => createComment.mutateAsync({ anchor, body })}
                          onReply={(commentId, body) => replyComment.mutateAsync({ commentId, body })}
                          onSetStatus={(commentId, status) => setCommentStatus.mutateAsync({ commentId, status })}
                          mutationPending={createComment.isPending || replyComment.isPending || setCommentStatus.isPending}
                        />
                      </div>
                    );
                  })}

                  {outdatedThreads.length > 0 && (
                    <section className="rounded-lg border border-dashed border-border bg-background p-3">
                      <h3 className="text-xs font-semibold">{t("diff.outdated")}</h3>
                      <p className="mt-1 text-xs text-muted-foreground">{t("diff.outdatedDetail")}</p>
                      <div className="mt-3 space-y-2">
                        {outdatedThreads.map((comment) => (
                          <div key={comment.id} className="rounded-md bg-muted/50 px-3 py-2 text-xs">
                            <span className="font-mono text-muted-foreground">
                              {comment.kind.kind === "thread" ? `${comment.kind.anchor.path}:${comment.kind.anchor.startLine}` : ""}
                            </span>
                            <p className="mt-1 whitespace-pre-wrap">{comment.body}</p>
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
              className={`${fileTreeOpen ? "" : "hidden"} z-10 transition-colors hover:bg-ring focus-visible:bg-ring`}
            />
            <ResizablePanel
              id="task-diff-files"
              panelRef={fileTreePanelRef}
              className="flex min-h-0 overflow-hidden"
              style={{ height: "100%", overflow: "hidden" }}
              defaultSize={fileTreeOpen ? 240 : 0}
              minSize={200}
              maxSize={400}
              collapsible
              collapsedSize={0}
              groupResizeBehavior="preserve-pixel-size"
              onResize={(size) => {
                const open = size.inPixels > 0;
                if (open !== fileTreeOpen) onFileTreeOpenChange(open);
              }}
            >
              {fileTreeOpen && (
                <TaskDiffFileTree
                  files={files}
                  selectedPath={activeFilePath}
                  onSelect={(path) => {
                    setSelectedAnchor(null);
                    setSelectedFilePath(path);
                    const root = scrollContainerRef.current;
                    const element = fileElementsRef.current.get(path);
                    if (root === null || element === undefined) return;
                    const top = element.getBoundingClientRect().top
                      - root.getBoundingClientRect().top
                      + root.scrollTop
                      - 16;
                    root.scrollTo({ top: Math.max(0, top), behavior: "smooth" });
                  }}
                />
              )}
            </ResizablePanel>
          </ResizablePanelGroup>
        )}
      </div>
      <CommitChangesDialog
        open={commitOpen}
        message={commitMessage}
        pending={commitChanges.isPending}
        error={commitChanges.error}
        onOpenChange={setCommitOpen}
        onMessageChange={setCommitMessage}
        onCommit={() => commitChanges.mutateAsync(commitMessage)}
      />
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

interface CommitChangesDialogProps {
  open: boolean;
  message: string;
  pending: boolean;
  error: Error | null;
  onOpenChange: (open: boolean) => void;
  onMessageChange: (message: string) => void;
  onCommit: () => Promise<unknown>;
}

/** Collects an explicit commit message before staging and committing task changes. */
function CommitChangesDialog({
  open,
  message,
  pending,
  error,
  onOpenChange,
  onMessageChange,
  onCommit,
}: CommitChangesDialogProps) {
  const { t } = useTranslation();
  return (
    <Dialog open={open} onOpenChange={(nextOpen) => !pending && onOpenChange(nextOpen)}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t("diff.commitDialogTitle")}</DialogTitle>
          <DialogDescription>{t("diff.commitDialogDescription")}</DialogDescription>
        </DialogHeader>
        <Textarea
          autoFocus
          rows={3}
          value={message}
          onChange={(event) => onMessageChange(event.target.value)}
          placeholder={t("diff.commitMessagePlaceholder")}
          aria-label={t("diff.commitMessage")}
          disabled={pending}
          className="resize-none"
        />
        {error !== null && <p className="text-xs text-destructive">{error.message}</p>}
        <DialogFooter>
          <Button variant="ghost" disabled={pending} onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            disabled={pending || message.trim() === ""}
            onClick={() => void onCommit()}
          >
            <IconGitCommit />{pending ? t("diff.committing") : t("diff.commit")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
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
    <AlertDialog open={open} onOpenChange={(nextOpen) => !pending && onOpenChange(nextOpen)}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("diff.pushDialogTitle")}</AlertDialogTitle>
          <AlertDialogDescription>{t("diff.pushDialogDescription")}</AlertDialogDescription>
        </AlertDialogHeader>
        {error !== null && <p className="text-xs text-destructive">{error.message}</p>}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={pending}>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction disabled={pending} onClick={() => void onPush()}>
            <IconUpload />{pending ? t("diff.pushing") : t("diff.push")}
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
  comments: TaskDiffComment[];
  reviewEnabled: boolean;
  selectedAnchor: SelectedAnchor | null;
  onSelectAnchor: (selection: SelectedAnchor | null) => void;
  onCreateComment: (anchor: TaskDiffCommentAnchor, body: string) => Promise<unknown>;
  onReply: (commentId: string, body: string) => Promise<unknown>;
  onSetStatus: (commentId: string, status: TaskDiffThreadStatus) => Promise<unknown>;
  mutationPending: boolean;
}

/** Renders one parsed patch file and injects review widgets below matching lines. */
function TaskDiffFile({
  file,
  fileIndex,
  viewType,
  diffId,
  comments,
  reviewEnabled,
  selectedAnchor,
  onSelectAnchor,
  onCreateComment,
  onReply,
  onSetStatus,
  mutationPending,
}: TaskDiffFileProps) {
  const { t } = useTranslation();
  const [expandedBlocks, setExpandedBlocks] = useState<Set<string>>(() => new Set());
  const fileStats = countChanges([file]);
  const renderSegments = useMemo(
    () => buildCollapsedDiffSegments(file.hunks, expandedBlocks),
    [expandedBlocks, file.hunks],
  );
  const threads = comments.filter((comment) => comment.kind.kind === "thread");
  const replies = comments.filter((comment) => comment.kind.kind === "reply");
  const selectedChangeKey = selectedAnchor?.changeKey.startsWith(`${fileIndex}:`)
    ? selectedAnchor.changeKey.slice(`${fileIndex}:`.length)
    : null;

  const widgets = Object.fromEntries(
    file.hunks.flatMap((hunk) =>
      hunk.changes.flatMap((change) => {
        const changeKey = getChangeKey(change);
        const oldLine = lineNumberFor(change, "old");
        const newLine = lineNumberFor(change, "new");
        const matchingThreads = threads.filter((thread) => {
          if (thread.kind.kind !== "thread") return false;
          const { anchor } = thread.kind;
          const expectedPath = anchor.side === "old" ? file.oldPath : file.newPath;
          const expectedLine = anchor.side === "old" ? oldLine : newLine;
          return anchor.path === expectedPath && anchor.startLine === expectedLine;
        });
        const isSelected = selectedChangeKey === changeKey;
        if (matchingThreads.length === 0 && !isSelected) return [];

        return [[changeKey, (
          <div className="space-y-2">
            {matchingThreads.map((thread) => (
              <DiffThread
                key={thread.id}
                thread={thread}
                replies={replies.filter(
                  (reply) => reply.kind.kind === "reply" && reply.kind.parentCommentId === thread.id,
                )}
                onReply={onReply}
                onSetStatus={onSetStatus}
                disabled={mutationPending}
              />
            ))}
            {isSelected && selectedAnchor !== null && (
              <CommentComposer
                anchor={selectedAnchor.anchor}
                onCancel={() => onSelectAnchor(null)}
                onSubmit={(body) => onCreateComment(selectedAnchor.anchor, body)}
                disabled={mutationPending}
              />
            )}
          </div>
        )] as const];
      }),
    ),
  );

  const selectLine = ({ change, side }: { change: ChangeData | null; side?: TaskDiffSide }) => {
    if (!reviewEnabled || change === null) return;
    const resolvedSide = resolveSide(change, side);
    const lineNumber = lineNumberFor(change, resolvedSide);
    if (lineNumber === null) return;
    const hunk = file.hunks.find((candidate) => candidate.changes.includes(change));
    if (hunk === undefined) return;

    onSelectAnchor({
      changeKey: `${fileIndex}:${getChangeKey(change)}`,
      anchor: createCommentAnchor(file, hunk, change, resolvedSide, diffId),
    });
  };

  return (
    <article className="bg-background">
      <header className="sticky top-0 z-10 flex min-h-10 items-center gap-2 border-b border-border/60 bg-background/95 px-2 py-2 backdrop-blur">
        <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-violet-500/12 text-violet-700 ring-1 ring-inset ring-violet-500/15 dark:text-violet-300">
          <IconFileDiff className="size-3.5" />
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-xs" title={displayPath(file)}>
          {displayPath(file)}
        </span>
        <span className="shrink-0 text-xs tabular-nums text-emerald-600">
          +{fileStats.additions}
        </span>
        <span className="shrink-0 text-xs tabular-nums text-red-600">
          −{fileStats.deletions}
        </span>
      </header>
      {file.hunks.length === 0 ? (
        <div className="px-4 py-8 text-center text-xs text-muted-foreground">
          {file.isBinary ? t("diff.binary") : t("diff.metadataOnly")}
        </div>
      ) : (
        <div
          className={`ora-task-diff ora-task-diff--${viewType} ora-task-diff--${file.type} overflow-x-auto`}
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
            selectedChanges={selectedChangeKey === null ? [] : [selectedChangeKey]}
            gutterEvents={{ onClick: selectLine }}
            renderGutter={viewType === "unified" ? renderSingleLineNumber : undefined}
            optimizeSelection
          >
            {() => renderSegments.map((segment) =>
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
                    aria-label={t("diff.expandUnchanged", { count: segment.lineCount })}
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
                    {t("diff.unchangedLinesHidden", { count: segment.lineCount })}
                  </button>
                </Decoration>
              )
            )}
          </Diff>
        </div>
      )}
    </article>
  );
}

interface DiffThreadProps {
  thread: TaskDiffComment;
  replies: TaskDiffComment[];
  onReply: (commentId: string, body: string) => Promise<unknown>;
  onSetStatus: (commentId: string, status: TaskDiffThreadStatus) => Promise<unknown>;
  disabled: boolean;
}

/** Displays one root review discussion, its replies, and lifecycle controls. */
function DiffThread({ thread, replies, onReply, onSetStatus, disabled }: DiffThreadProps) {
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
        <Badge variant={thread.kind.status === "open" ? "secondary" : "outline"} className="text-[10px]">
          {thread.kind.status}
        </Badge>
        <div className="flex-1" />
        <Button
          size="xs"
          variant="ghost"
          disabled={disabled}
          onClick={() => void onSetStatus(thread.id, nextStatus)}
        >
          <IconCheck />{nextStatus === "resolved" ? t("diff.resolve") : t("diff.reopen")}
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
          <Button size="sm" disabled={disabled || reply.trim() === ""} onClick={() => void submitReply()}>
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
function CommentComposer({ anchor, onCancel, onSubmit, disabled }: CommentComposerProps) {
  const { t } = useTranslation();
  const [body, setBody] = useState("");

  const submit = async () => {
    const comment = body.trim();
    if (comment === "") return;
    await onSubmit(comment);
  };

  return (
    <section className="rounded-md border border-primary/30 bg-background p-3 text-xs shadow-sm">
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
        className="resize-y text-xs"
      />
      <div className="mt-2 flex justify-end gap-2">
        <Button size="sm" variant="ghost" disabled={disabled} onClick={onCancel}>{t("common.cancel")}</Button>
        <Button size="sm" disabled={disabled || body.trim() === ""} onClick={() => void submit()}>
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
      <span role="status" className="sr-only">{t("diff.loading")}</span>
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

/** Counts inserted and deleted lines across parsed patch files. */
export function countChanges(files: FileData[]): DiffStats {
  return files.reduce(
    (total, file) => file.hunks.reduce(
      (fileTotal, hunk) => hunk.changes.reduce(
        (hunkTotal, change) => ({
          additions: hunkTotal.additions + (change.type === "insert" ? 1 : 0),
          deletions: hunkTotal.deletions + (change.type === "delete" ? 1 : 0),
        }),
        fileTotal,
      ),
      total,
    ),
    { additions: 0, deletions: 0 },
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

/** Builds the exact single-line anchor shape validated by the backend patch parser. */
export function createCommentAnchor(
  file: FileData,
  hunk: HunkData,
  change: ChangeData,
  side: TaskDiffSide,
  diffId: string,
): TaskDiffCommentAnchor {
  const lineNumber = lineNumberFor(change, side);
  if (lineNumber === null) {
    throw new Error(`change ${getChangeKey(change)} does not exist on the ${side} side`);
  }

  return {
    diffId,
    path: side === "old" ? file.oldPath : file.newPath,
    side,
    startLine: lineNumber,
    endLine: lineNumber,
    hunkHeader: hunk.content,
    // gitdiff-parser retains the CR from CRLF patches, while Rust `str::lines`
    // deliberately removes it before validating the source line.
    lineContent: change.content.replace(/\r$/, ""),
  };
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
  if (change.type === "delete") return side === "old" ? change.lineNumber : null;
  return side === "new" ? change.lineNumber : null;
}

/** Chooses the path users expect for added, deleted, and renamed files. */
function displayPath(file: FileData): string {
  return file.type === "delete" ? file.oldPath : file.newPath;
}
