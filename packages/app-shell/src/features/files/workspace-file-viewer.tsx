import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { ScrollArea } from "@ora/ui";
import type { BundledLanguage, ThemedTokenWithVariants } from "shiki";
import { useVirtualizer, type VirtualItem } from "@tanstack/react-virtual";
import { useTranslation } from "react-i18next";
import { workspaceFileVisual } from "./workspace-file-visuals";
import {
  lineDisplayColumns,
  utf8ByteColumnToStringIndex,
} from "./workspace-file-viewer-utils";
import {
  useQuoteLineSelection,
  type QuoteLineAnchor,
} from "../quote-line-selection";
import "./workspace-file-viewer.css";
import "../quote-line-selection.css";

const MAX_HIGHLIGHT_BYTES = 512 * 1024;

/** Rendered row height: `text-xs leading-5`, one non-wrapping line per row. */
const LINE_HEIGHT_PX = 20;
/**
 * Files up to this many lines render every row: the DOM cost is trivial and
 * interactions keep working across off-screen lines (pinned washes stay
 * painted without re-render help). Beyond it only a window of rows is
 * mounted — a full render of a multi-megabyte file costs hundreds of
 * thousands of DOM nodes, which blocks every session switch that remounts
 * the preview panel.
 */
const VIRTUALIZE_MIN_LINES = 400;
/** Extra rows above/below the window so fast wheel scrolling rarely shows gaps. */
const VIRTUAL_OVERSCAN = 24;
/**
 * Widest line counted for the scrollable width, in columns. Without the cap a
 * previewed minified bundle (one multi-million-column line) would request an
 * element wider than browsers allow; anything past the cap clips horizontally.
 */
const MAX_MEASURED_COLUMNS = 100_000;
/** Fixed viewer chrome in front of the line text: 3.5rem gutter + 0.75rem padding on both sides. */
const ROW_CHROME_REM = 5;
/**
 * Files above this many characters defer every per-content pass (split,
 * quote lookups, width scan) until after the first paint and show the
 * loading overlay instead. Session switches remount this panel inside the
 * switch's own render, so without the deferral those passes block the new
 * session page from appearing at all. The cost scales with characters, not
 * lines, so this is measured on the raw content.
 */
const DEFER_MOUNT_CHARS = 512_000;
/**
 * Cache-key separator for the highlight cache; language ids never contain
 * NUL, so this separator cannot collide with a language prefix.
 */
const HIGHLIGHT_KEY_SEPARATOR = "\u0000";

export interface WorkspaceFileMatchTarget {
  line: number;
  column: number;
  matchedText: string;
  /** Inclusive end of a cited range; omitted for a single line or search match. */
  endLine?: number;
}

interface WorkspaceFileViewerProps {
  content: string;
  path: string;
  target: WorkspaceFileMatchTarget | null;
  /** Clears the Files header jump label after a citation wash is dismissed. */
  onDismissJump?: () => void;
}

interface ShikiTokenStyle extends CSSProperties {
  "--shiki-dark"?: string;
}

const highlightedFileCache = new Map<
  string,
  Promise<ThemedTokenWithVariants[][] | null>
>();

interface HighlightedFile {
  key: string;
  tokens: ThemedTokenWithVariants[][] | null;
}

/** Renders UTF-8 text with stable line numbers and scrolls selected search matches into view. */
export function WorkspaceFileViewer({
  content,
  path,
  target,
  onDismissJump,
}: WorkspaceFileViewerProps) {
  const { t } = useTranslation();
  const targetRow = useRef<HTMLSpanElement | null>(null);
  // Large files mount in two steps: the first commit renders the loading
  // overlay with no rows and every per-content pass below short-circuited,
  // then a post-paint timer brings the real content in. The timer is a
  // macrotask, so the browser paints the session page with the overlay before
  // the split/scan work blocks the main thread. Keyed on mount only: a
  // refetch of the same file must not flash the overlay over live content.
  const [deferred, setDeferred] = useState(
    () => content.length > DEFER_MOUNT_CHARS,
  );
  const lines = useMemo(
    () => (deferred ? [] : content.split(/\r\n|\n|\r/)),
    [deferred, content],
  );
  const language = workspaceFileVisual(path).language;
  // Infinity while deferred keeps `highlightEnabled` false so the lazy
  // highlighter never receives the not-yet-split content.
  const contentByteLength = useMemo(
    () =>
      deferred
        ? Number.POSITIVE_INFINITY
        : new TextEncoder().encode(content).byteLength,
    [deferred, content],
  );
  const highlightEnabled =
    !deferred && contentByteLength <= MAX_HIGHLIGHT_BYTES;

  useEffect(() => {
    if (!deferred) return;
    const timer = setTimeout(() => setDeferred(false), 0);
    return () => clearTimeout(timer);
  }, [deferred]);
  const highlightKey = highlightEnabled
    ? `${language}${HIGHLIGHT_KEY_SEPARATOR}${content}`
    : null;
  const [highlighted, setHighlighted] = useState<HighlightedFile | null>(null);

  const anchors = useMemo<QuoteLineAnchor[]>(
    () =>
      lines.map((_line, index) => ({
        key: String(index + 1),
        lineNumber: index + 1,
        path,
      })),
    [lines, path],
  );

  const {
    rootRef,
    pinnedRange,
    onGutterMouseDown,
    onPlusMouseDown,
    onPlusClick,
    onNumberClick,
    onNumberKeyDown,
  } = useQuoteLineSelection({ anchors });
  // Jump/search wash is only a locate-then-read cue. Remembering the dismissed
  // target object (not a boolean) means a later jump with a new object paints
  // again without an effect to reset state.
  const [dismissedTarget, setDismissedTarget] =
    useState<WorkspaceFileMatchTarget | null>(null);
  const highlightTarget =
    target !== null && Object.is(target, dismissedTarget) ? null : target;
  const isSearchMatch =
    highlightTarget !== null && highlightTarget.matchedText.length > 0;
  const citationStart = isSearchMatch ? undefined : highlightTarget?.line;
  const citationEnd =
    isSearchMatch || highlightTarget === null
      ? undefined
      : (highlightTarget.endLine ?? highlightTarget.line);
  const citationLo =
    citationStart === undefined || citationEnd === undefined
      ? undefined
      : Math.min(citationStart, citationEnd);
  const citationHi =
    citationStart === undefined || citationEnd === undefined
      ? undefined
      : Math.max(citationStart, citationEnd);

  const virtualize = lines.length >= VIRTUALIZE_MIN_LINES;
  const preRef = useRef<HTMLPreElement | null>(null);
  // Base UI's ScrollArea owns the real scroll element between the root and
  // the <pre>, so the viewport has to be discovered from the mounted DOM.
  const [viewport, setViewport] = useState<HTMLDivElement | null>(null);
  // The list starts below the <pre>'s vertical padding; every scroll
  // computation must add that offset (tanstack's `scrollMargin`).
  const [scrollMargin, setScrollMargin] = useState(0);

  useLayoutEffect(() => {
    setViewport(
      preRef.current?.closest<HTMLDivElement>(
        '[data-slot="scroll-area-viewport"]',
      ) ?? null,
    );
  }, []);

  useLayoutEffect(() => {
    const pre = preRef.current;
    if (viewport === null || pre === null) return;
    setScrollMargin(
      pre.getBoundingClientRect().top -
        viewport.getBoundingClientRect().top +
        viewport.scrollTop,
    );
  }, [viewport]);

  // eslint-disable-next-line react-hooks/incompatible-library -- the virtualizer instance is consumed imperatively (scrollToIndex, getTotalSize) during render; compiler memoization of this component is not required
  const virtualizer = useVirtualizer({
    count: lines.length,
    enabled: virtualize && viewport !== null,
    getScrollElement: () => viewport,
    estimateSize: () => LINE_HEIGHT_PX,
    overscan: VIRTUAL_OVERSCAN,
    scrollMargin,
    // Line numbers never reorder within one file, so window shifts reuse rows
    // instead of swapping their text.
    getItemKey: (index) => index + 1,
  });

  // Width of the widest line: with every row mounted min-w-max establishes the
  // scroll range, but a mounted window cannot, so the monospace column
  // estimate keeps the horizontal scrollbar from resizing while scrolling
  // vertically.
  const scrollWidthCh = useMemo(() => {
    let max = 0;
    for (const line of lines) {
      const columns = lineDisplayColumns(line);
      if (columns > max) max = columns;
    }
    return Math.min(max, MAX_MEASURED_COLUMNS);
  }, [lines]);

  useEffect(() => {
    let active = true;
    if (highlightKey === null) {
      return () => {
        active = false;
      };
    }
    let pending = highlightedFileCache.get(highlightKey);
    if (pending === undefined) {
      pending = import("shiki")
        .then(({ codeToTokensWithThemes }) =>
          codeToTokensWithThemes(content, {
            lang: language as BundledLanguage,
            themes: { light: "light-plus", dark: "dark-plus" },
          }),
        )
        .catch(() => null);
      highlightedFileCache.set(highlightKey, pending);
    }
    pending.then((nextTokens) => {
      if (active) setHighlighted({ key: highlightKey, tokens: nextTokens });
    });
    return () => {
      active = false;
    };
  }, [content, highlightKey, language]);

  useEffect(() => {
    if (highlightTarget === null) return;
    if (virtualize) {
      // The target row is usually not mounted, so centering must go through
      // the virtualizer instead of scrollIntoView on an element.
      if (viewport !== null) {
        virtualizer.scrollToIndex(highlightTarget.line - 1, {
          align: "center",
        });
      }
      return;
    }
    targetRow.current?.scrollIntoView({ block: "center", inline: "nearest" });
  }, [
    content,
    highlightTarget,
    scrollMargin,
    virtualize,
    virtualizer,
    viewport,
  ]);

  const renderRow = (line: string, index: number, item: VirtualItem | null) => {
    const lineNumber = index + 1;
    const inCitedRange =
      citationLo !== undefined &&
      citationHi !== undefined &&
      lineNumber >= citationLo &&
      lineNumber <= citationHi;
    // Pin washes are re-applied declaratively here because a windowed file
    // mounts rows after the click that pinned them; the hook's imperative
    // paint only reached the rows visible at click time.
    const isPinned =
      pinnedRange !== null &&
      lineNumber >= pinnedRange.startLine &&
      lineNumber <= pinnedRange.endLine;
    const isSearchLine = isSearchMatch && highlightTarget.line === lineNumber;
    const isScrollTarget =
      isSearchLine || (inCitedRange && lineNumber === citationStart);
    const match = isSearchLine
      ? matchRange(line, highlightTarget.column, highlightTarget.matchedText)
      : null;
    return (
      <span
        key={lineNumber}
        ref={isScrollTarget ? targetRow : undefined}
        aria-current={isScrollTarget ? "location" : undefined}
        data-line-number={lineNumber}
        data-quote-key={lineNumber}
        data-cited-range={inCitedRange ? "true" : undefined}
        data-quote-pinned={isPinned ? "true" : undefined}
        className={`workspace-file-line group/line relative block ${isSearchLine ? "bg-amber-500/10" : ""}`}
        style={
          item === null
            ? undefined
            : {
                position: "absolute",
                top: item.start - scrollMargin,
                left: 0,
                width: "100%",
                height: item.size,
              }
        }
        onMouseDown={(event) => {
          if (event.button !== 0) return;
          if (
            event.target instanceof Element &&
            event.target.closest("[data-quote-gutter]")
          ) {
            onGutterMouseDown(event, String(lineNumber));
          }
        }}
      >
        <span
          data-quote-gutter
          className="workspace-file-gutter sticky left-0 z-[1] inline-flex h-5 select-none items-center justify-end bg-background"
        >
          <span
            data-quote-number
            role="button"
            tabIndex={0}
            aria-label={t("files.selectLine", { line: lineNumber })}
            aria-keyshortcuts="Control+Enter Meta+Enter"
            className="workspace-file-line-number inline-block min-w-[1.75rem] cursor-pointer text-right tabular-nums text-muted-foreground/65 group-hover/line:text-foreground"
            onClick={(event) => onNumberClick(event, String(lineNumber))}
            onKeyDown={(event) => onNumberKeyDown(event, String(lineNumber))}
          >
            {lineNumber}
          </span>
          <button
            type="button"
            tabIndex={-1}
            data-quote-button
            className="workspace-file-quote-btn"
            aria-label={t("files.quoteLineToChat", { line: lineNumber })}
            onMouseDown={(event) => onPlusMouseDown(event, String(lineNumber))}
            onClick={(event) => onPlusClick(event, String(lineNumber))}
          />
        </span>
        <span className="px-3">
          {renderHighlightedLine(
            line,
            highlighted?.key === highlightKey
              ? highlighted.tokens?.[index]
              : undefined,
            match,
          )}
        </span>
      </span>
    );
  };

  // A windowed file renders no rows until the scroll viewport is discovered —
  // including the very first commit. Falling back to a full render while the
  // viewport is not ready yet would re-create every row inside the session
  // switch's own commit, which is exactly the stall this windowing exists to
  // avoid. The layout effect resolves the viewport before paint, so the empty
  // window never becomes visible.
  const viewerReady = !virtualize || viewport !== null;
  const rows = !viewerReady
    ? []
    : virtualize
      ? virtualizer
          .getVirtualItems()
          .map((item) => renderRow(lines[item.index]!, item.index, item))
      : lines.map((line, index) => renderRow(line, index, null));

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      {!deferred && !highlightEnabled && (
        <p
          data-large-file-notice
          className="shrink-0 border-b border-border bg-muted/30 px-3 py-1.5 text-[11px] text-muted-foreground"
        >
          {t("files.largeFilePlainText")}
        </p>
      )}
      <ScrollArea className="min-h-0 flex-1" scrollbars="both">
        <pre
          ref={(node) => {
            rootRef.current = node;
            preRef.current = node;
          }}
          data-quote-root
          data-selectable
          className="workspace-file-viewer min-w-max py-4 font-mono text-xs leading-5 text-foreground"
          onMouseDown={(event) => {
            if (event.button !== 0 || highlightTarget === null) return;
            // Search matches stay until the user picks another result.
            if (isSearchMatch) return;
            if (!(event.target instanceof Element)) return;
            if (event.target.closest("button") !== null) return;
            const hit = event.target.closest("[data-cited-range='true']");
            if (hit === null) {
              setDismissedTarget(target);
              onDismissJump?.();
            }
          }}
        >
          <code
            style={
              virtualize
                ? {
                    display: "block",
                    position: "relative",
                    height: virtualizer.getTotalSize(),
                    width: `calc(${ROW_CHROME_REM}rem + ${scrollWidthCh}ch)`,
                  }
                : undefined
            }
          >
            {rows}
          </code>
        </pre>
      </ScrollArea>
      {(deferred || !viewerReady) && (
        <div
          data-deferred-loading
          className="absolute inset-0 z-10 flex items-center justify-center bg-background/60 text-sm text-muted-foreground"
        >
          {t("files.loading")}
        </div>
      )}
    </div>
  );
}

/** Combines Shiki token colors with the exact ripgrep match marker for one line. */
function renderHighlightedLine(
  line: string,
  tokens: ThemedTokenWithVariants[] | undefined,
  match: { start: number; end: number } | null,
): ReactNode {
  if (tokens === undefined) return renderPlainLine(line, match);
  if (match === null) return renderTokenRange(tokens, 0, line.length, "line");

  return (
    <>
      {renderTokenRange(tokens, 0, match.start, "before")}
      <mark className="rounded-sm bg-amber-300/70 px-0 text-inherit dark:bg-amber-500/45">
        {renderTokenRange(tokens, match.start, match.end, "match")}
      </mark>
      {renderTokenRange(tokens, match.end, line.length, "after")}
    </>
  );
}

/** Keeps the file immediately readable while the lazy syntax highlighter is loading. */
function renderPlainLine(
  line: string,
  match: { start: number; end: number } | null,
): ReactNode {
  if (match === null) return line;
  return (
    <>
      {line.slice(0, match.start)}
      <mark className="rounded-sm bg-amber-300/70 px-0 text-inherit dark:bg-amber-500/45">
        {line.slice(match.start, match.end)}
      </mark>
      {line.slice(match.end)}
    </>
  );
}

/** Slices themed tokens by UTF-16 offsets so one semantic match can keep a single marker. */
function renderTokenRange(
  tokens: ThemedTokenWithVariants[],
  start: number,
  end: number,
  keyPrefix: string,
): ReactNode[] {
  const rendered: ReactNode[] = [];
  let offset = 0;
  for (const [index, token] of tokens.entries()) {
    const tokenStart = offset;
    const tokenEnd = tokenStart + token.content.length;
    offset = tokenEnd;
    const sliceStart = Math.max(start, tokenStart);
    const sliceEnd = Math.min(end, tokenEnd);
    if (sliceStart >= sliceEnd) continue;

    const light = token.variants.light;
    const dark = token.variants.dark;
    const style: ShikiTokenStyle = {
      color: light?.color,
      "--shiki-dark": dark?.color,
    };
    rendered.push(
      <span
        key={`${keyPrefix}-${index}-${sliceStart}`}
        className="shiki-token"
        style={style}
      >
        {token.content.slice(sliceStart - tokenStart, sliceEnd - tokenStart)}
      </span>,
    );
  }
  return rendered;
}

/** Finds the exact ripgrep match span without treating a regular-expression query as literal text. */
function matchRange(
  line: string,
  column: number,
  matchedText: string,
): { start: number; end: number } | null {
  const start = utf8ByteColumnToStringIndex(line, column);
  if (matchedText.length === 0 || !line.startsWith(matchedText, start))
    return null;
  return { start, end: start + matchedText.length };
}
