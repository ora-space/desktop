import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  Decoration,
  Diff,
  Hunk,
  getChangeKey,
  type FileData,
  type GutterOptions,
} from "react-diff-view";
import {
  IconChevronDown,
  IconFileDiff,
  IconLayoutSidebarRightExpand,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import {
  buildCollapsedDiffSegments,
  buildRenderChunks,
  chunkIndexForChange,
  estimateChunkHeight,
  findDiffLineTargets,
  type DiffRenderChunk,
  type DiffRenderSegment,
} from "./task-diff-collapse";
import { countChanges } from "./task-diff-data";
import { createDiffLineAligner, type DiffLineAligner } from "./task-diff-jump";
import { useTaskDiffQuoteGutter } from "./task-diff-quote-gutter";

/**
 * Changed lines above this mount the file's diff in two steps: the first
 * commit renders a placeholder, then a post-paint timer brings the rows in.
 * Session switches remount this panel inside the switch's own render, so a
 * multi-thousand-line rewrite would otherwise be created there and block the
 * new page from painting. Mount-only: a refetch of the same patch never
 * re-defers.
 */
export const DEFER_MOUNT_CHANGES = 600;

/**
 * A file with more changed lines than this switches the body to the row-windowed
 * render: the segments are split into native `react-diff-view` hunks and only
 * the visible chunk window is mounted, so a multi-thousand-line file scrolls
 * with bounded DOM while keeping every gutter/tint/collapse behavior.
 */
export const ROW_VIRTUALIZE_MIN_CHANGES = 400;

export interface TaskDiffFileProps {
  file: FileData;
  viewType: "unified" | "split";
  targetLine?: number;
  targetEndLine?: number;
  targetSide?: "old" | "new";
  /**
   * The scroll region that holds this file. When provided and the file is large
   * enough (see `ROW_VIRTUALIZE_MIN_CHANGES`), the body is row-windowed into
   * native chunk hunks instead of one full table; every state/effect (jump,
   * quote, collapse) keeps working because the windowing lives inside this
   * component, not in a sidecar.
   */
  scrollElement?: HTMLElement | null;
  /**
   * Whether the diff file tree is currently open. When it is closed the file
   * header shows an expand button at its right end (the tree's own header keeps
   * the collapse action).
   */
  fileTreeOpen?: boolean;
  /** Re-opens the collapsed diff file tree; the button only shows while closed. */
  onExpandFileTree?: () => void;
}

/** Renders one parsed patch file. */
export function TaskDiffFile({
  file,
  viewType,
  targetLine,
  targetEndLine,
  targetSide = "new",
  scrollElement,
  fileTreeOpen = true,
  onExpandFileTree,
}: TaskDiffFileProps) {
  const { t } = useTranslation();
  const fileRootRef = useRef<HTMLElement | null>(null);
  // Single-file (focus) mode: the whole pane shows one file, so a per-file
  // collapse toggle is redundant — hide it and keep the header as a plain
  // title. The continuous scroll body (no scrollElement) keeps the collapse.
  const isSingleFile = scrollElement !== undefined;
  // Citation-jump aligner: waits (bounded, on rAF) for the cited row to mount
  // in windowed mode, then vertically centers it. Held in a ref so a new jump
  // on the same pane replaces the previous poll instead of competing with it.
  // The root/region is the file `<article>`; the scroll region and the selected
  // row are discovered from the DOM so this stays framework-free.
  const jumpAlignerRef = useRef<DiffLineAligner | null>(null);
  if (jumpAlignerRef.current === null) {
    jumpAlignerRef.current = createDiffLineAligner({
      getRegion: () => {
        const region = fileRootRef.current?.closest<HTMLElement>(
          ".ora-diff-scroll-region",
        );
        return region ?? null;
      },
      getRow: () =>
        fileRootRef.current?.querySelector<HTMLElement>(
          ".diff-code-selected, .diff-selected",
        ) ?? null,
    });
  }
  const [expanded, setExpanded] = useState(true);
  const [expandedBlocks, setExpandedBlocks] = useState<Set<string>>(
    () => new Set(),
  );
  const changeCount = useMemo(
    () => file.hunks.reduce((total, hunk) => total + hunk.changes.length, 0),
    [file.hunks],
  );
  const [deferred, setDeferred] = useState(
    () => changeCount > DEFER_MOUNT_CHANGES,
  );

  useEffect(() => {
    if (!deferred) return;
    const timer = setTimeout(() => setDeferred(false), 0);
    return () => clearTimeout(timer);
  }, [deferred]);

  // Stop the jump-align poll if this file unmounts mid-align, so it never keeps
  // scrolling a node that has been removed from the tree.
  useEffect(
    () => () => {
      jumpAlignerRef.current?.cancel();
    },
    [],
  );

  const { renderGutter, quoteRootRef } = useTaskDiffQuoteGutter(file, viewType);
  const fileStats = useMemo(() => countChanges([file]), [file]);
  const jumpTargets = useMemo(
    () =>
      targetLine === undefined
        ? []
        : findDiffLineTargets(
            file.hunks,
            targetLine,
            targetEndLine ?? targetLine,
            targetSide,
          ),
    [file.hunks, targetEndLine, targetLine, targetSide],
  );
  const selectedChanges = useMemo(
    () => jumpTargets.map((target) => getChangeKey(target.change)),
    [jumpTargets],
  );
  const jumpScrollKey = selectedChanges[0] ?? null;
  if (targetLine !== undefined && !expanded) {
    setExpanded(true);
  }
  const collapsedKeysToExpand = jumpTargets
    .map((target) => target.collapsedKey)
    .filter((key): key is string => key !== null && !expandedBlocks.has(key));
  if (collapsedKeysToExpand.length > 0) {
    const next = new Set(expandedBlocks);
    for (const key of collapsedKeysToExpand) next.add(key);
    setExpandedBlocks(next);
  }
  const renderSegments = useMemo(
    () => buildCollapsedDiffSegments(file.hunks, expandedBlocks),
    [expandedBlocks, file.hunks],
  );
  // Row-window a large file: split the segments into native chunk hunks and
  // mount only the visible window. Windowing lives here (not a sidecar) so the
  // jump/quote/collapse effects below keep working for both render paths.
  const windowed =
    scrollElement !== undefined &&
    scrollElement !== null &&
    changeCount > ROW_VIRTUALIZE_MIN_CHANGES;
  const chunks = useMemo(
    () => (windowed ? buildRenderChunks(renderSegments) : null),
    [renderSegments, windowed],
  );
  // The virtualizer instance is consumed render-style (getTotalSize) and the
  // chunk window recomputes on every scroll; compiler memoization of this
  // component is intentionally skipped because its inputs already feed a
  // `useMemo`-stable chunks list.
  // eslint-disable-next-line react-hooks/incompatible-library -- scroll input is a DOM element and the window is a render-time concern
  const virtualizer = useVirtualizer({
    count: chunks?.length ?? 0,
    enabled: windowed,
    getScrollElement: () => scrollElement ?? null,
    estimateSize: (index) => estimateChunkHeight(chunks![index]!),
    // Overscan in chunks (each ~a viewport tall), not rows.
    overscan: 2,
    getItemKey: (index) => chunks![index]!.key,
    // The jump effect writes scrollTop and dispatches a scroll event from inside
    // a layout effect; the virtualizer would otherwise `flushSync` a re-render
    // there, and React rejects a flush inside a lifecycle ("flushSync was called
    // from inside a lifecycle method"), which trips the clean-stderr test gate.
    // A queued (non-sync) re-render is visually identical here, so opt out.
    useFlushSync: false,
  });
  // Jump target for the cited change, and the scroll offset of its chunk so the
  // windowed body can scroll the region directly (a native scrollTop write the
  // virtualizer observes) instead of relying on `scrollToIndex`. The offset is
  // the estimated height of every chunk before the target's; the final exact
  // alignment comes from centering the highlighted row once its chunk mounts.
  const { chunkIndex, chunkOffset } = useMemo(() => {
    if (chunks === null || jumpTargets.length === 0) {
      return { chunkIndex: -1, chunkOffset: 0 };
    }
    const index = chunkIndexForChange(chunks, jumpTargets[0]!.change);
    if (index < 0) return { chunkIndex: -1, chunkOffset: 0 };
    let offset = 0;
    for (let i = 0; i < index; i += 1) {
      offset += estimateChunkHeight(chunks[i]!);
    }
    return { chunkIndex: index, chunkOffset: offset };
  }, [chunks, jumpTargets]);

  useLayoutEffect(() => {
    if (jumpScrollKey === null) return;
    if (
      chunks !== null &&
      chunkIndex >= 0 &&
      scrollElement !== null &&
      scrollElement !== undefined
    ) {
      // Row-windowed: the cited change may live in a later chunk that is not
      // mounted until the virtualizer scrolls there. Scroll the region to the
      // target chunk's offset directly — a native scrollTop write that the
      // virtualizer observes and re-windows from — then the aligner waits for
      // the row to mount and centers it.
      scrollElement.scrollTop = chunkOffset;
      scrollElement.dispatchEvent(new Event("scroll"));
    }
    // (Re)start the aligner; it cancels its own previous poll, so repeated
    // citation jumps on the same pane each replace the last instead of missing.
    jumpAlignerRef.current?.align();
  }, [chunkIndex, chunkOffset, chunks, jumpScrollKey, scrollElement]);

  const expandCollapsed = useCallback((key: string) => {
    setExpandedBlocks((current) => {
      const next = new Set(current);
      next.add(key);
      return next;
    });
  }, []);

  const body =
    file.hunks.length === 0 ? (
      <div className="px-4 py-8 text-center text-xs text-muted-foreground">
        {file.isBinary ? t("diff.binary") : t("diff.metadataOnly")}
      </div>
    ) : deferred ? (
      <div
        data-diff-deferred-loading
        role="status"
        className="flex items-center justify-center text-xs text-muted-foreground"
        style={{ minHeight: diffFileEstimatedHeight(file) }}
      >
        {t("diff.loading")}
      </div>
    ) : windowed && chunks !== null ? (
      <div
        ref={(node) => {
          quoteRootRef.current = node;
        }}
        data-quote-root
        className={`ora-task-diff ora-task-diff--${viewType} relative overflow-x-auto`}
        style={{ height: virtualizer.getTotalSize() }}
      >
        {virtualizer.getVirtualItems().map((virtualItem) => {
          const chunk = chunks[virtualItem.index]!;
          return (
            <div
              key={virtualItem.key}
              data-index={virtualItem.index}
              className="ora-diff-window-chunk absolute left-0 top-0 w-full"
              ref={(node) => {
                // Measure real height so the scroll track matches the native
                // tables exactly (no gaps where the estimate is off).
                virtualizer.measureElement(node);
                if (node !== null) {
                  node.style.transform = `translateY(${virtualItem.start}px)`;
                }
              }}
            >
              {renderWindowedChunk(
                chunk,
                viewType,
                file.type,
                renderGutter,
                selectedChanges,
                t,
                expandCollapsed,
              )}
            </div>
          );
        })}
      </div>
    ) : (
      <div
        ref={(node) => {
          quoteRootRef.current = node;
        }}
        data-quote-root
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
          selectedChanges={selectedChanges}
          renderGutter={renderGutter}
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
                    onClick={() => expandCollapsed(segment.key)}
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
    );

  const expandFileTreeButton =
    !fileTreeOpen && onExpandFileTree !== undefined ? (
      <button
        type="button"
        className="ml-2 flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
        aria-label={t("diff.expandFileTree")}
        title={t("diff.expandFileTree")}
        onClick={onExpandFileTree}
      >
        <IconLayoutSidebarRightExpand className="size-3.5" />
      </button>
    ) : null;

  return (
    <article ref={fileRootRef} className="bg-background">
      <header className="sticky top-0 z-10 border-b border-border/60 bg-background/95 backdrop-blur">
        {isSingleFile ? (
          <div className="flex min-h-10 w-full items-center gap-2 px-2 py-2">
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
            {expandFileTreeButton}
          </div>
        ) : (
          <div className="flex min-h-10 w-full items-center">
            <button
              type="button"
              className="flex min-h-10 min-w-0 flex-1 items-center gap-2 px-2 py-2 text-left outline-none transition-colors hover:bg-muted/35 focus-visible:bg-muted/35 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
              aria-expanded={expanded}
              aria-label={t(
                expanded ? "diff.collapseFile" : "diff.expandFile",
                {
                  path: displayPath(file),
                },
              )}
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
            {expandFileTreeButton}
          </div>
        )}
      </header>
      {expanded && body}
    </article>
  );
}

/** Chooses the path users expect for added, deleted, and renamed files. */
function displayPath(file: FileData): string {
  return file.type === "delete" ? file.oldPath : file.newPath;
}

/** Compares one file's render inputs so sibling files can skip work. */
function areTaskDiffFilePropsEqual(
  previous: TaskDiffFileProps,
  next: TaskDiffFileProps,
): boolean {
  return (
    previous.file === next.file &&
    previous.viewType === next.viewType &&
    previous.targetLine === next.targetLine &&
    previous.targetEndLine === next.targetEndLine &&
    previous.targetSide === next.targetSide &&
    previous.scrollElement === next.scrollElement &&
    previous.fileTreeOpen === next.fileTreeOpen &&
    previous.onExpandFileTree === next.onExpandFileTree
  );
}

/** Placeholder height for a not-yet-rendered diff, close enough to avoid scroll jumps. */
function diffFileEstimatedHeight(file: FileData): number {
  return Math.max(
    72,
    48 +
      file.hunks.reduce((total, hunk) => total + hunk.changes.length, 0) * 24,
  );
}

/**
 * Renders one row-windowed chunk through native `react-diff-view` components so
 * the gutter/tint/collapse styling is identical to the non-windowed path. A
 * hunk chunk renders as a standalone `<Diff>` (with its own `<Hunk>`); a folded
 * chunk renders the collapsed-block `<Decoration>`.
 */
function renderWindowedChunk(
  chunk: DiffRenderChunk,
  viewType: "unified" | "split",
  diffType: FileData["type"],
  renderGutter: (options: GutterOptions) => ReactNode,
  selectedChanges: string[],
  t: (key: string, opts?: Record<string, unknown>) => string,
  onExpandCollapsed: (key: string) => void,
): ReactNode {
  if (chunk.kind === "collapsed") {
    return (
      <div className="diff-decoration ora-diff-collapsed">
        <div className="diff-decoration-content ora-diff-collapsed-cell">
          <button
            type="button"
            className="group flex h-8 w-full items-center justify-center gap-2 text-[11px] text-muted-foreground outline-none transition-colors hover:bg-violet-500/8 hover:text-foreground focus-visible:bg-violet-500/10 focus-visible:text-foreground"
            aria-label={t("diff.expandUnchanged", { count: chunk.lineCount })}
            onClick={() => onExpandCollapsed(chunk.key)}
          >
            <span className="flex size-5 items-center justify-center rounded-md bg-violet-500/10 text-violet-700 transition-colors group-hover:bg-violet-500/15 dark:text-violet-300">
              <IconChevronDown className="size-3.5" />
            </span>
            {t("diff.unchangedLinesHidden", { count: chunk.lineCount })}
          </button>
        </div>
      </div>
    );
  }
  return (
    <Diff
      viewType={viewType}
      diffType={diffType}
      hunks={[chunk.hunk]}
      selectedChanges={selectedChanges}
      renderGutter={renderGutter}
      optimizeSelection
    >
      {() => <Hunk key={chunk.key} hunk={chunk.hunk} />}
    </Diff>
  );
}

export const MemoizedTaskDiffFile = memo(
  TaskDiffFile,
  areTaskDiffFilePropsEqual,
);

/** Keeps the previous render-segment type importable without exposing it. */
export type { DiffRenderSegment };
