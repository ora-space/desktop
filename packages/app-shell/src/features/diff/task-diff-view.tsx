import {
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
import { type FileData } from "react-diff-view";
import "react-diff-view/style/index.css";
import "./task-diff-view.css";
import type { WorkspaceDiffScope } from "@ora/contracts";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  type ResizablePanelHandle,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
} from "@ora/ui";
import {
  IconCode,
  IconGitBranch,
  IconRefresh,
  IconUpload,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { useContractsClient } from "../../contracts-client-context";
import { localizeContractError } from "../../i18n/contract-error";
import { invalidateWorkspaceDiffs } from "../../state/data/diff";
import { useWorkspaceDiff } from "../../state/hooks/use-workspace-diff";
import {
  countChanges,
  DEFER_PARSE_PATCH_CHARS,
  parseTaskDiffSegment,
  splitPatchSegments,
} from "./task-diff-data";
import { diffFilePath } from "./task-diff-file-tree-utils";
import { MemoizedTaskDiffFile, type TaskDiffFileProps } from "./task-diff-file";
import { TaskDiffFileTree } from "./task-diff-file-tree";
import { TaskGitActions } from "./task-git-actions";
import { TaskDiffFocusBody } from "./task-diff-focus-view";
import {
  animatePanelWidth,
  cancelPanelWidthAnimation,
} from "../../lib/panel-motion";
import { pathsMatchForWorkspace } from "../../lib/workspace-path";
import {
  fileNavigationLocation,
  type FileNavigationLocation,
} from "./task-changes-navigation-context";
import { isDiffScrollAtEnd } from "./task-diff-scroll";
import {
  runDiffFileScroll,
  type DiffFileScrollRunHandle,
} from "./task-diff-scroll-run";
import { TaskDiffScrollArea } from "./task-diff-scroll-area";

/** Matches the changes-panel slide so the file tree toggle feels consistent. */
const FILE_TREE_SLIDE_MS = 180;
const FILE_TREE_WIDTH = 240;
/** Narrowest tree width a user resize settles on; below it the tree collapses. */
const FILE_TREE_MIN_WIDTH = 180;
const FILE_TREE_COLLAPSE_THRESHOLD = FILE_TREE_MIN_WIDTH / 2;

/**
 * Parsed `FileData` keyed by the per-file patch slice, so a live-sync
 * invalidation that rewrites only a few files returns the same `FileData`
 * reference for unchanged slices and the sibling `memo` comparison skips them
 * on re-render. Module-level (not a ref) because it is read during render; the
 * patch slices it keys are already retained by the react-query cache, so its
 * growth is bounded by the patches the session has loaded.
 */
const segmentFileCache = new Map<string, FileData>();

interface TaskDiffViewProps {
  workspaceId: string;
  /**
   * Whether this workspace has a recorded baseline commit (an isolated task
   * worktree does; a project's main checkout does not). Gates the `Branch`/
   * `Committed` scopes, which compare against that baseline.
   */
  hasBaseline: boolean;
  viewType: TaskDiffViewType;
  fileTreeOpen: boolean;
  fileRequest?: TaskDiffFileRequest;
  toolbar?: ReactNode;
  onFileTreeOpenChange: (open: boolean) => void;
  onFileNotFound?: (path: string, location?: FileNavigationLocation) => void;
  /** Reports the file currently shown so review layout can persist it. */
  onPreviewPathChange?: (path: string) => void;
}

export type TaskDiffViewType = "unified" | "split";

export interface TaskDiffFileRequest {
  path: string;
  requestId: number;
  line?: number;
  /** Inclusive end of a cited range; omitted for a single-line jump. */
  endLine?: number;
  /** Patch side the line numbers belong to; omitted for new-side chat links. */
  side?: "old" | "new";
}

/**
 * A single file with this many changed lines switches to the row-windowed body
 * (see `task-diff-virtualized-file.tsx`), which only runs under the single-file
 * focus body. Auto-focus therefore also triggers when any one file reaches this
 * size, so a lone huge change-set is focused and windowed rather than rendered
 * as one long native table.
 */
const ROW_VIRTUALIZE_MIN_CHANGES = 400;
/**
 * Auto-selects the focused (single-file) body above these sizes. The scroll
 * body maps every file into one DOM list, so a high file count, a large
 * change-set, or a single file big enough to row-window (above
 * `ROW_VIRTUALIZE_MIN_CHANGES`) is what makes the single-file body worth it. A
 * manual toggle sticks for the component lifetime, so `auto` or `scroll` only
 * flips until the user picks; it is not re-evaluated on file count changes.
 */
const FOCUS_MODE_FILE_COUNT = 25;
const FOCUS_MODE_TOTAL_CHANGES = 2000;

/** Renders a task worktree patch. */
export function TaskDiffView({
  workspaceId,
  hasBaseline,
  viewType,
  fileTreeOpen,
  fileRequest,
  toolbar,
  onFileTreeOpenChange,
  onFileNotFound,
  onPreviewPathChange,
}: TaskDiffViewProps) {
  const { i18n, t } = useTranslation();
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const [scope, setScope] = useState<WorkspaceDiffScope>(
    hasBaseline ? "branch" : "unstaged",
  );
  const [gitActionsOpen, setGitActionsOpen] = useState(false);
  const [commitMessage, setCommitMessage] = useState("");
  const [pushOpen, setPushOpen] = useState(false);
  const [gitNotice, setGitNotice] = useState<string | null>(null);
  const diffQuery = useWorkspaceDiff(workspaceId, scope);
  const [selectedFilePath, setSelectedFilePath] = useState<string | null>(null);
  const [appliedFileRequestId, setAppliedFileRequestId] = useState<
    number | null
  >(null);
  // Jump wash is a locate-then-read cue. Storing the dismissed requestId
  // (instead of a boolean reset in an effect) lets the next chat jump paint
  // again as soon as requestId changes.
  const [dismissedJumpRequestId, setDismissedJumpRequestId] = useState<
    number | null
  >(null);
  const jumpHighlightDismissed =
    fileRequest !== undefined &&
    fileRequest.requestId === dismissedJumpRequestId;
  /**
   * Shared jump-wash dismissal: a left-click on a non-cited, non-chrome region
   * clears the highlighted jump so the next chat citation can repaint. Used by
   * both the continuous scroll body and the single-file focus body (windowed),
   * so clicking elsewhere in the diff always dismisses the highlight.
   */
  const dismissJumpHighlight = useCallback(
    (event: React.MouseEvent) => {
      if (
        event.button !== 0 ||
        jumpHighlightDismissed ||
        fileRequest?.line === undefined ||
        !(event.target instanceof Element)
      ) {
        return;
      }
      const onCitedRow =
        event.target.closest(".diff-code-selected, .diff-selected") !== null;
      const onChrome = event.target.closest("button") !== null;
      if (!onCitedRow && !onChrome) {
        setDismissedJumpRequestId(fileRequest.requestId);
      }
    },
    [fileRequest, jumpHighlightDismissed],
  );
  // Stable so passing it into `MemoizedTaskDiffFile` does not defeat its memo:
  // an inline arrow would change identity on every render and force every
  // mounted file to re-render its expensive `<Diff>` subtree.
  const openFileTree = useCallback(() => {
    onFileTreeOpenChange(true);
  }, [onFileTreeOpenChange]);
  // Last tree-click flash. It is never cleared by a timer: the overlay fades
  // to transparent via `animation-fill-mode`, and re-clicking re-keys it to
  // replay — so a click on an already-visible file still gets feedback.
  const [fileFlash, setFileFlash] = useState<{
    path: string;
    seq: number;
  } | null>(null);
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const fileElementsRef = useRef(new Map<string, HTMLDivElement>());
  const fileTreePanelRef = useRef<ResizablePanelHandle | null>(null);
  const fileTreeAnimationRef = useRef<number | null>(null);
  const fileTreeWidthRef = useRef(FILE_TREE_WIDTH);
  // The single scroll run that currently owns the viewport. While it is set
  // the scroll spy stands down, so the run's own scroll events and the layout
  // shifts it triggers cannot steal the selection mid-flight.
  const activeScrollRunRef = useRef<DiffFileScrollRunHandle | null>(null);
  /** File request whose jump scroll already started, deduplicating effect re-runs. */
  const jumpScrollRequestIdRef = useRef<number | null>(null);
  const onPreviewPathChangeRef = useRef(onPreviewPathChange);
  /** Last path reported upward, so repeat notifications collapse to one call. */
  const notifiedPreviewPathRef = useRef<string | null>(null);

  useEffect(() => {
    onPreviewPathChangeRef.current = onPreviewPathChange;
  });

  /**
   * Reports the previewed file to the review layout.
   *
   * Must stay callable from plain event/effect code only — never from a
   * `setState` updater, which React may re-run and which must not touch another
   * component's state.
   */
  const notifyPreviewPath = useCallback((path: string) => {
    if (notifiedPreviewPathRef.current === path) return;
    notifiedPreviewPathRef.current = path;
    onPreviewPathChangeRef.current?.(path);
  }, []);

  /**
   * Two-step parse (A1): a patch above `DEFER_PARSE_PATCH_CHARS` returns an
   * empty list on the first render and lets the session page paint, then a
   * post-paint macro-task parses. Mount-only — a refetch never re-defers.
   */
  const patch = diffQuery.data?.patch ?? "";
  const [parseDeferred, setParseDeferred] = useState(
    () => patch.length > DEFER_PARSE_PATCH_CHARS,
  );
  useEffect(() => {
    if (!parseDeferred) return;
    const timer = setTimeout(() => setParseDeferred(false), 0);
    return () => clearTimeout(timer);
  }, [parseDeferred]);

  const files = useMemo(() => {
    if (patch.length === 0 || parseDeferred) return [];
    const parsed: FileData[] = [];
    for (const segment of splitPatchSegments(patch)) {
      const cached = segmentFileCache.get(segment);
      if (cached !== undefined) {
        parsed.push(cached);
        continue;
      }
      const file = parseTaskDiffSegment(segment);
      if (file !== undefined) {
        segmentFileCache.set(segment, file);
        parsed.push(file);
      }
    }
    return parsed;
  }, [patch, parseDeferred]);
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

  /**
   * View mode is driven entirely by the diff size: a large change-set (many
   * files, many changed lines, or one file big enough to row-window) uses the
   * single-file focus body, otherwise the continuous scroll body. The focus
   * body windows a huge file into native chunk hunks, so it is the only layout
   * that stays fast for a big diff; the scroll body mounts every file as one
   * full table and cannot window per-file. Derived (not ref-stored) so the
   * deferred parse flip below also gates the mode correctly.
   */
  const largestFileChanges = useMemo(
    () =>
      files.reduce(
        (max, file) =>
          Math.max(
            max,
            file.hunks.reduce((total, hunk) => total + hunk.changes.length, 0),
          ),
        0,
      ),
    [files],
  );
  const autoFocus =
    files.length > FOCUS_MODE_FILE_COUNT ||
    stats.additions + stats.deletions > FOCUS_MODE_TOTAL_CHANGES ||
    largestFileChanges > ROW_VIRTUALIZE_MIN_CHANGES;
  const isFocusMode = autoFocus;
  const focusFile =
    isFocusMode && activeFilePath !== ""
      ? (files.find((file) => diffFilePath(file) === activeFilePath) ?? null)
      : null;

  // Deferring the parse leaves `filePaths` empty for one render. Do not mark
  // the request applied yet, or `onFileNotFound` (a chat jump that could not
  // resolve a path) fires while the patch is still parsing and yanks the
  // layout into the Files panel. Once the parse lands the block re-runs.
  if (
    fileRequest !== undefined &&
    !diffQuery.isLoading &&
    !parseDeferred &&
    fileRequest.requestId !== appliedFileRequestId
  ) {
    setAppliedFileRequestId(fileRequest.requestId);
    const matchingPath = filePaths.find((path) =>
      pathsMatchForWorkspace(fileRequest.path, path),
    );
    if (matchingPath !== undefined) {
      setSelectedFilePath(matchingPath);
    }
  }

  useLayoutEffect(() => {
    if (fileRequest === undefined || diffQuery.isLoading) return;
    if (fileRequest.requestId !== appliedFileRequestId) return;
    const matchingPath = filePaths.find((path) =>
      pathsMatchForWorkspace(fileRequest.path, path),
    );
    if (matchingPath !== undefined) {
      notifyPreviewPath(matchingPath);
    }
  }, [
    appliedFileRequestId,
    diffQuery.isLoading,
    filePaths,
    fileRequest,
    notifyPreviewPath,
  ]);

  /**
   * Scrolls `path` to the top of the Changes viewport and keeps re-aligning it
   * while virtualized placeholders take their real height. The run holds the
   * scroll spy until it settles — or the user takes over — so neither the
   * run's own scroll events nor late layout shifts can move the selection
   * elsewhere. A new run always replaces the previous one.
   */
  const startScrollRun = useCallback((path: string) => {
    activeScrollRunRef.current?.cancel();
    const run = runDiffFileScroll({
      getRoot: () => scrollContainerRef.current,
      getTarget: () => fileElementsRef.current.get(path),
      onArrived: () => {
        if (activeScrollRunRef.current === run) {
          activeScrollRunRef.current = null;
        }
      },
      onInterrupted: () => {
        if (activeScrollRunRef.current === run) {
          activeScrollRunRef.current = null;
        }
      },
    });
    activeScrollRunRef.current = run;
  }, []);

  // A run outliving this panel must not keep scrolling against detached nodes.
  useEffect(() => () => activeScrollRunRef.current?.cancel(), []);

  /** Selects a changed file; in scroll mode also aligns its header with the top. */
  const selectFile = useCallback(
    (path: string) => {
      setSelectedFilePath(path);
      notifyPreviewPath(path);
      if (!isFocusMode) startScrollRun(path);
      setFileFlash((current) => ({ path, seq: (current?.seq ?? 0) + 1 }));
    },
    [isFocusMode, notifyPreviewPath, startScrollRun],
  );

  useLayoutEffect(() => {
    if (isFocusMode) return;
    if (fileRequest === undefined || diffQuery.isLoading) return;
    if (fileRequest.requestId !== appliedFileRequestId) return;
    // One jump per request: the run itself owns retries, so selection changes
    // and refetch-driven list identity changes must not re-jump the viewport
    // to an old request's path (they would yank the user away from wherever
    // they scrolled — visible as a double run when tree-clicking the very
    // file a restored request points at).
    if (jumpScrollRequestIdRef.current === fileRequest.requestId) return;
    const matchingPath = filePaths.find((path) =>
      pathsMatchForWorkspace(fileRequest.path, path),
    );
    if (matchingPath === undefined || matchingPath !== selectedFilePath) return;
    jumpScrollRequestIdRef.current = fileRequest.requestId;
    startScrollRun(matchingPath);
  }, [
    appliedFileRequestId,
    diffQuery.isLoading,
    filePaths,
    fileRequest,
    isFocusMode,
    selectedFilePath,
    startScrollRun,
  ]);

  useEffect(() => {
    if (fileRequest === undefined || diffQuery.isLoading || parseDeferred) {
      return;
    }
    if (fileRequest.requestId !== appliedFileRequestId) return;
    const matchingPath = filePaths.find((path) =>
      pathsMatchForWorkspace(fileRequest.path, path),
    );
    if (matchingPath === undefined) {
      onFileNotFound?.(
        fileRequest.path,
        fileNavigationLocation({
          line: fileRequest.line,
          endLine: fileRequest.endLine,
          side: fileRequest.side,
        }),
      );
    }
  }, [
    fileRequest,
    appliedFileRequestId,
    filePaths,
    diffQuery.isLoading,
    onFileNotFound,
    parseDeferred,
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
    if (isFocusMode) return;
    const root = scrollContainerRef.current;
    if (root === null || filePaths.length === 0) return;
    let frame: number | null = null;
    const updateActiveFile = () => {
      // A programmatic jump owns the viewport until it settles; skipping (not
      // consuming anything) keeps its own scroll events and the layout shifts
      // they trigger from moving the selection mid-flight.
      if (activeScrollRunRef.current !== null) return;
      if (frame !== null) return;
      frame = requestAnimationFrame(() => {
        frame = null;
        let activePath = filePaths[0]!;
        if (isDiffScrollAtEnd(root)) {
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
        // Notify outside the updater: updaters must be pure, and this one
        // would otherwise setState on the parent review layout.
        notifyPreviewPath(activePath);
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
  }, [filePaths, isFocusMode, notifyPreviewPath]);

  const commitChanges = useMutation({
    mutationFn: (message: string) =>
      client.workspace.commitChanges({ workspaceId, message }),
    onSuccess: async (response) => {
      setGitActionsOpen(false);
      setCommitMessage("");
      setGitNotice(t("diff.commitSucceeded", { summary: response.summary }));
      // A baseline-less workspace has no fixed "committed" comparison to show;
      // its remaining uncommitted changes are still the most useful view.
      setScope(hasBaseline ? "committed" : "unstaged");
      await invalidateWorkspaceDiffs(queryClient, workspaceId);
    },
  });
  const pushBranch = useMutation({
    mutationFn: () => client.workspace.pushBranch({ workspaceId }),
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
    await diffQuery.refetch();
  };

  if (diffQuery.isLoading) {
    return <DiffLoadingState />;
  }

  if (diffQuery.error !== null) {
    const error = diffQuery.error;
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

  const mutationError = commitChanges.error ?? pushBranch.error;

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
              setScope(value as WorkspaceDiffScope);
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
              {hasBaseline && (
                <SelectItem value="branch">{t("diff.scopeBranch")}</SelectItem>
              )}
              <SelectItem value="unstaged">
                {t("diff.scopeUnstaged")}
              </SelectItem>
              <SelectItem value="staged">{t("diff.scopeStaged")}</SelectItem>
              {hasBaseline && (
                <SelectItem value="committed">
                  {t("diff.scopeCommitted")}
                </SelectItem>
              )}
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
        <div className="ora-diff-toolbar__view-controls flex min-w-0 shrink items-center gap-0.5">
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
          parseDeferred ? (
            <DiffLoadingState />
          ) : (
            <DiffMessage
              title={t("diff.noChanges")}
              detail={t("diff.noChangesDetail")}
            />
          )
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
              {isFocusMode ? (
                <TaskDiffFocusBody
                  file={focusFile}
                  viewType={viewType}
                  fileTreeOpen={fileTreeOpen}
                  onExpandFileTree={openFileTree}
                  onDismissJumpHighlight={dismissJumpHighlight}
                  fileFlash={
                    fileFlash?.path === activeFilePath ? fileFlash : null
                  }
                  targetLine={
                    !jumpHighlightDismissed &&
                    fileRequest !== undefined &&
                    focusFile !== null &&
                    pathsMatchForWorkspace(
                      fileRequest.path,
                      diffFilePath(focusFile),
                    )
                      ? fileRequest.line
                      : undefined
                  }
                  targetEndLine={
                    !jumpHighlightDismissed &&
                    fileRequest !== undefined &&
                    focusFile !== null &&
                    pathsMatchForWorkspace(
                      fileRequest.path,
                      diffFilePath(focusFile),
                    )
                      ? fileRequest.endLine
                      : undefined
                  }
                  targetSide={
                    !jumpHighlightDismissed &&
                    fileRequest !== undefined &&
                    focusFile !== null &&
                    pathsMatchForWorkspace(
                      fileRequest.path,
                      diffFilePath(focusFile),
                    )
                      ? fileRequest.side
                      : undefined
                  }
                />
              ) : (
                <TaskDiffScrollArea
                  viewportRef={scrollContainerRef}
                  onMouseDown={dismissJumpHighlight}
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
                          className="relative scroll-mt-0"
                        >
                          <TaskDiffFileViewport
                            file={file}
                            viewType={viewType}
                            fileTreeOpen={fileTreeOpen}
                            onExpandFileTree={openFileTree}
                            targetLine={
                              !jumpHighlightDismissed &&
                              fileRequest !== undefined &&
                              pathsMatchForWorkspace(fileRequest.path, path)
                                ? fileRequest.line
                                : undefined
                            }
                            targetEndLine={
                              !jumpHighlightDismissed &&
                              fileRequest !== undefined &&
                              pathsMatchForWorkspace(fileRequest.path, path)
                                ? fileRequest.endLine
                                : undefined
                            }
                            targetSide={
                              !jumpHighlightDismissed &&
                              fileRequest !== undefined &&
                              pathsMatchForWorkspace(fileRequest.path, path)
                                ? fileRequest.side
                                : undefined
                            }
                            rootRef={scrollContainerRef}
                            forceRender={activeFilePath === path}
                          />
                          {fileFlash?.path === path && (
                            <div
                              key={fileFlash.seq}
                              aria-hidden="true"
                              className="ora-diff-file-flash"
                            />
                          )}
                        </div>
                      );
                    })}
                  </div>
                </TaskDiffScrollArea>
              )}
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
                  onSelect={selectFile}
                  onCollapse={() => onFileTreeOpenChange(false)}
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

interface TaskDiffFileViewportProps extends TaskDiffFileProps {
  rootRef: RefObject<HTMLDivElement | null>;
  forceRender: boolean;
}

/** Placeholder height for a not-yet-rendered diff, close enough to avoid scroll jumps. */
function diffFileEstimatedHeight(file: FileData): number {
  return Math.max(
    72,
    48 +
      file.hunks.reduce((total, hunk) => total + hunk.changes.length, 0) * 24,
  );
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
  const estimatedHeight = useMemo(() => diffFileEstimatedHeight(file), [file]);

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

interface DiffMessageProps {
  title: string;
  detail: string;
  action?: ReactNode;
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
