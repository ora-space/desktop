import { useCallback, useEffect, useMemo, useRef } from "react";
import { addComposerFileSelections } from "./chat/add-composer-file-selection";

/**
 * One quoteable row. `key` must be unique within the rendered surface: file
 * previews use the line number, diff gutters use `${side}:${line}` so the old
 * and new columns of a split view never collide.
 */
export interface QuoteLineAnchor {
  key: string;
  lineNumber: number;
  snippet: string;
  path: string;
  /** Drag lock group (diff side). Undefined means a single flat surface. */
  group?: string;
  /** Diff-gutter quotes; file previews omit this. */
  origin?: "diff";
  diffSide?: "old" | "new";
}

export interface QuoteLineSelection {
  path: string;
  startLine: number;
  endLine: number;
  snippet: string;
  origin?: "diff";
  /** Present when every quoted line is the same side; mixed add/delete omits it. */
  diffSide?: "old" | "new";
}

interface QuoteDragState {
  startKey: string;
  endKey: string;
  group: string | undefined;
  /** Split view: stay on one side. Unified: follow visual rows across sides. */
  lockToGroup: boolean;
  dragged: boolean;
}

interface UseQuoteLineSelectionOptions<T extends QuoteLineAnchor> {
  anchors: readonly T[];
  /** When false, the + control and drag-quote are disabled (pin stays). */
  enabled?: boolean;
  /**
   * When true (split diff), a drag never crosses old/new columns. Unified
   * view passes false so a delete-row press can continue onto inserts below.
   */
  lockToGroup?: boolean;
}

/**
 * Shared gutter interaction for the file preview and the task diff:
 * hover reveals a `+`, clicking it quotes one line, pressing the gutter and
 * dragging vertically quotes a range, and clicking (or Enter/Space on) the
 * number pins a (shift-extendable, same-side) selection highlight.
 *
 * All mid-gesture feedback is imperative (`data-quote-*` attributes) so the
 * pressed control is never remounted and large files never re-render per
 * mousemove. React only learns about the final commit, which goes straight to
 * the composer store. Unified diffs range by anchor order so a drag that
 * starts on a delete can continue through inserts below and still insert one
 * chip; split diffs stay locked to one side and fill by line number
 * (including collapsed gaps).
 */
export function useQuoteLineSelection<T extends QuoteLineAnchor>({
  anchors,
  enabled = true,
  lockToGroup = false,
}: UseQuoteLineSelectionOptions<T>) {
  const rootRef = useRef<HTMLElement | null>(null);
  const dragRef = useRef<QuoteDragState | null>(null);
  /** Rows currently tinted by the active drag (key → cells), so a mousemove only touches rows that entered or left the range. */
  const paintedRef = useRef(new Map<string, HTMLElement[]>());
  const anchorElRef = useRef<HTMLElement | null>(null);
  const pinnedRef = useRef<{
    group: string | undefined;
    anchorLine: number;
    startLine: number;
    endLine: number;
  } | null>(null);
  const suppressClickRef = useRef(false);

  /**
   * Every lookup the gesture handlers need, built in one pass so a surface as
   * large as a whole file preview (one anchor per line) does not walk its
   * anchors three times: `byKey` carries the anchor plus its render index,
   * `byLine` is the `${group ?? ""}:${lineNumber}` index used for range fills.
   */
  const lookups = useMemo(() => {
    const byKey = new Map<string, { anchor: T; index: number }>();
    const byLine = new Map<string, T>();
    for (let index = 0; index < anchors.length; index += 1) {
      const anchor = anchors[index]!;
      byKey.set(anchor.key, { anchor, index });
      byLine.set(`${anchor.group ?? ""}:${anchor.lineNumber}`, anchor);
    }
    return { byKey, byLine };
  }, [anchors]);

  const anchorForKey = useCallback(
    (key: string): T | undefined => lookups.byKey.get(key)?.anchor,
    [lookups],
  );

  const anchorForLine = useCallback(
    (group: string | undefined, line: number): T | undefined =>
      lookups.byLine.get(`${group ?? ""}:${line}`),
    [lookups],
  );

  /** Inclusive slice of anchors in render order between two keys. */
  const anchorsBetween = useCallback(
    (startKey: string, endKey: string): T[] => {
      const start = lookups.byKey.get(startKey)?.index;
      const end = lookups.byKey.get(endKey)?.index;
      if (start === undefined || end === undefined) return [];
      return anchors.slice(Math.min(start, end), Math.max(start, end) + 1);
    },
    [anchors, lookups],
  );

  /** Anchors between two line numbers on one side; lines with none are skipped. */
  const anchorsInLineRange = useCallback(
    (group: string | undefined, lo: number, hi: number): T[] => {
      const found: T[] = [];
      for (let line = lo; line <= hi; line += 1) {
        const next = anchorForLine(group, line);
        if (next !== undefined) found.push(next);
      }
      return found;
    },
    [anchorForLine],
  );

  /** Paints exactly `wanted` keys; only newly entered rows cost a DOM query. */
  const paintKeys = useCallback((wanted: ReadonlySet<string>) => {
    const root = rootRef.current;
    if (root === null) return;
    const painted = paintedRef.current;
    for (const [key, els] of painted) {
      if (!wanted.has(key)) {
        markQuoteAttr(els, "data-quote-selected", false);
        painted.delete(key);
      }
    }
    for (const key of wanted) {
      if (painted.has(key)) continue;
      const el = root.querySelector<HTMLElement>(`[data-quote-key="${key}"]`);
      if (el === null) continue;
      const targets = quotePaintTargets(el);
      markQuoteAttr(targets, "data-quote-selected", true);
      painted.set(key, targets);
    }
  }, []);

  /** Paints exactly the rows in [lo, hi] that have anchors; only newly entered rows cost a DOM query. */
  const paintRange = useCallback(
    (group: string | undefined, lo: number, hi: number) => {
      paintKeys(
        new Set(anchorsInLineRange(group, lo, hi).map((anchor) => anchor.key)),
      );
    },
    [anchorsInLineRange, paintKeys],
  );

  const clearDragPaint = useCallback(() => {
    for (const els of paintedRef.current.values()) {
      markQuoteAttr(els, "data-quote-selected", false);
    }
    paintedRef.current = new Map();
    anchorElRef.current?.removeAttribute("data-quote-anchor");
    anchorElRef.current = null;
  }, []);

  const clearPinned = useCallback(() => {
    const root = rootRef.current;
    pinnedRef.current = null;
    root
      ?.querySelectorAll("[data-quote-pinned]")
      .forEach((el) => el.removeAttribute("data-quote-pinned"));
  }, []);

  const cancelDrag = useCallback(() => {
    dragRef.current = null;
    rootRef.current?.removeAttribute("data-quote-dragging");
    clearDragPaint();
  }, [clearDragPaint]);

  const beginDrag = useCallback(
    (key: string) => {
      if (!enabled) return;
      const anchor = anchorForKey(key);
      if (anchor === undefined) return;
      // A fresh press owns the next click, so no earlier gesture's suppression
      // can still be standing when that click arrives.
      suppressClickRef.current = false;
      window.getSelection()?.removeAllRanges();
      // The pinned selection is kept until the pointer actually travels:
      // mousedown precedes every click, and shift-click on a number must
      // still see the previous pin to extend from it.
      dragRef.current = {
        startKey: key,
        endKey: key,
        group: anchor.group,
        lockToGroup,
        dragged: false,
      };
      rootRef.current?.setAttribute("data-quote-dragging", "true");
      paintKeys(new Set([key]));
      const el = rootRef.current?.querySelector<HTMLElement>(
        `[data-quote-key="${key}"]`,
      );
      // Anchor the + on the keyed gutter overlay, not the <tr>: split rows
      // share a tr across both sides.
      anchorElRef.current = el ?? null;
      el?.setAttribute("data-quote-anchor", "true");
    },
    [anchorForKey, enabled, lockToGroup, paintKeys],
  );

  const quote = useCallback(
    (key: string) => {
      if (!enabled) return;
      const anchor = anchorForKey(key);
      if (anchor === undefined) return;
      addComposerFileSelections([selectionFromAnchors([anchor])]);
    },
    [anchorForKey, enabled],
  );

  useEffect(() => {
    /** Commits the dragged range. File gaps still split chips; a Diff drag stays one chip. */
    const finishDrag = () => {
      const state = dragRef.current;
      if (state === null) return;
      cancelDrag();
      if (!state.dragged) return;
      // The click that follows a drag-ending mouseup on the same + must not
      // quote again. The flag is consumed by that click and cleared by the next
      // press, so it can neither outlive the gesture nor depend on timer
      // ordering against the click event.
      suppressClickRef.current = true;
      let inRange: T[];
      if (state.lockToGroup) {
        const startLine = anchorForKey(state.startKey)?.lineNumber;
        const endLine = anchorForKey(state.endKey)?.lineNumber;
        inRange =
          startLine === undefined || endLine === undefined
            ? []
            : anchorsInLineRange(
                state.group,
                Math.min(startLine, endLine),
                Math.max(startLine, endLine),
              );
      } else {
        inRange = anchorsBetween(state.startKey, state.endKey);
      }
      const selections = buildQuoteSelections(inRange);
      if (selections.length > 0) addComposerFileSelections(selections);
    };

    const trackDrag = (event: MouseEvent) => {
      const state = dragRef.current;
      if (state === null) return;
      if (event.buttons !== 1) {
        // Button was released outside the window: never leave a stale drag.
        cancelDrag();
        return;
      }
      const root = rootRef.current;
      if (root === null) return;
      const key = quoteKeyFromPoint(
        event.clientX,
        event.clientY,
        root,
        state.lockToGroup && state.group !== undefined ? state.group : "any",
      );
      if (key === null) return;
      const anchor = anchorForKey(key);
      if (anchor === undefined) return;
      if (key === state.endKey) return;
      state.endKey = key;
      // High-water mark: returning to the start row keeps the drag a drag.
      state.dragged ||= key !== state.startKey;
      if (state.dragged) clearPinned();
      if (state.lockToGroup) {
        const startLine = anchorForKey(state.startKey)?.lineNumber;
        const endLine = anchor.lineNumber;
        if (startLine !== undefined) {
          paintRange(
            state.group,
            Math.min(startLine, endLine),
            Math.max(startLine, endLine),
          );
        }
      } else {
        paintKeys(
          new Set(
            anchorsBetween(state.startKey, state.endKey).map(
              (item) => item.key,
            ),
          ),
        );
      }
    };

    const cancelOnBlur = () => cancelDrag();

    window.addEventListener("mouseup", finishDrag, true);
    window.addEventListener("mousemove", trackDrag, true);
    window.addEventListener("blur", cancelOnBlur);
    return () => {
      window.removeEventListener("mouseup", finishDrag, true);
      window.removeEventListener("mousemove", trackDrag, true);
      window.removeEventListener("blur", cancelOnBlur);
    };
  }, [
    anchorForKey,
    anchorsBetween,
    anchorsInLineRange,
    cancelDrag,
    clearPinned,
    paintKeys,
    paintRange,
  ]);

  // Content switches under the same surface drop every painted marker.
  useEffect(() => {
    cancelDrag();
    clearPinned();
  }, [anchors, cancelDrag, clearPinned]);

  /** Gutter press starts a range drag; the + button handles its own press. */
  const onGutterMouseDown = useCallback(
    (event: React.MouseEvent<HTMLElement>, key: string) => {
      if (event.button !== 0) return;
      if (
        event.target instanceof Element &&
        event.target.closest("[data-quote-button]") !== null
      ) {
        return;
      }
      event.preventDefault();
      beginDrag(key);
    },
    [beginDrag],
  );

  /** Holding + starts the same drag; its click afterwards is suppressed. */
  const onPlusMouseDown = useCallback(
    (event: React.MouseEvent<HTMLElement>, key: string) => {
      if (event.button !== 0) return;
      event.preventDefault();
      beginDrag(key);
    },
    [beginDrag],
  );

  /**
   * True when this click is the tail of a drag that already quoted. Consuming
   * the flag here (rather than clearing it on a timer) keeps it scoped to
   * exactly one click, whichever mouse handler sees it first.
   */
  const consumeSuppressedClick = useCallback((): boolean => {
    if (!suppressClickRef.current) return false;
    suppressClickRef.current = false;
    return true;
  }, []);

  const onPlusClick = useCallback(
    (event: React.MouseEvent<HTMLElement>, key: string) => {
      event.preventDefault();
      event.stopPropagation();
      if (consumeSuppressedClick()) return;
      quote(key);
    },
    [consumeSuppressedClick, quote],
  );

  /** Number click pins a selection; shift extends from the previous pin. */
  const pinRange = useCallback(
    (group: string | undefined, anchorLine: number, endLine: number) => {
      const startLine = Math.min(anchorLine, endLine);
      const pinnedEnd = Math.max(anchorLine, endLine);
      pinnedRef.current = {
        group,
        anchorLine,
        startLine,
        endLine: pinnedEnd,
      };
      clearPinnedAttributes(rootRef.current);
      paintRange(group, startLine, pinnedEnd);
      for (const els of paintedRef.current.values()) {
        markQuoteAttr(els, "data-quote-pinned", true);
        markQuoteAttr(els, "data-quote-selected", false);
      }
      paintedRef.current = new Map();
    },
    [paintRange],
  );

  /** Click and keyboard share this so Enter matches the "select line" label. */
  const pinFromNumber = useCallback(
    (event: { shiftKey: boolean }, key: string) => {
      const anchor = anchorForKey(key);
      if (anchor === undefined) return;
      const previous = pinnedRef.current;
      const extend =
        event.shiftKey && previous !== null && previous.group === anchor.group;
      pinRange(
        anchor.group,
        extend && previous !== null ? previous.anchorLine : anchor.lineNumber,
        anchor.lineNumber,
      );
    },
    [anchorForKey, pinRange],
  );

  /**
   * Quotes the pinned range, or this line alone when nothing is pinned. The
   * line number is the only focusable control in the gutter — the `+` stays
   * out of the tab order so a long file does not add one tab stop per line —
   * so this is the keyboard's route to the same action the `+` performs.
   */
  const quoteFromNumber = useCallback(
    (key: string) => {
      if (!enabled) return;
      const anchor = anchorForKey(key);
      if (anchor === undefined) return;
      const pinned = pinnedRef.current;
      const inRange =
        pinned !== null && pinned.group === anchor.group
          ? anchorsInLineRange(pinned.group, pinned.startLine, pinned.endLine)
          : [anchor];
      const selections = buildQuoteSelections(inRange);
      if (selections.length > 0) addComposerFileSelections(selections);
    },
    [anchorForKey, anchorsInLineRange, enabled],
  );

  const onNumberClick = useCallback(
    (event: React.MouseEvent<HTMLElement>, key: string) => {
      if (consumeSuppressedClick()) return;
      pinFromNumber(event, key);
    },
    [consumeSuppressedClick, pinFromNumber],
  );

  /**
   * Enter/Space on the number pins (shift extends), matching click.
   * Ctrl/Cmd+Enter quotes the pinned range into the composer.
   */
  const onNumberKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLElement>, key: string) => {
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      if (event.ctrlKey || event.metaKey) {
        quoteFromNumber(key);
        return;
      }
      pinFromNumber(event, key);
    },
    [pinFromNumber, quoteFromNumber],
  );

  return {
    rootRef,
    onGutterMouseDown,
    onPlusMouseDown,
    onPlusClick,
    onNumberClick,
    onNumberKeyDown,
  };
}

/**
 * One chip for a visual run. Diff mixed add/delete still uses min–max line
 * numbers (Cursor-style `L1-5`) and the new-side path when any insert/context
 * row is present; `diffSide` is omitted when the run spans both sides.
 */
function selectionFromAnchors<T extends QuoteLineAnchor>(
  run: readonly T[],
): QuoteLineSelection {
  const first = run[0]!;
  let startLine = first.lineNumber;
  let endLine = first.lineNumber;
  for (const anchor of run) {
    if (anchor.lineNumber < startLine) startLine = anchor.lineNumber;
    if (anchor.lineNumber > endLine) endLine = anchor.lineNumber;
  }
  const newSide = run.find((anchor) => anchor.diffSide === "new");
  const selection: QuoteLineSelection = {
    path: newSide?.path ?? first.path,
    startLine,
    endLine,
    snippet: run.map((anchor) => anchor.snippet).join("\n"),
  };
  if (run.some((anchor) => anchor.origin === "diff")) {
    selection.origin = "diff";
    const sides = new Set<"old" | "new">();
    for (const anchor of run) {
      if (anchor.diffSide !== undefined) sides.add(anchor.diffSide);
    }
    if (sides.size === 1) {
      selection.diffSide = [...sides][0];
    }
  }
  return selection;
}

/**
 * Whether `anchor` belongs on the same composer chip as `previous`.
 * File quotes still split on path changes and skipped line numbers. A Diff
 * drag stays one chip even across add/delete and collapsed hunks — the
 * composer shows one range; +/- in the snippet is what the agent uses.
 */
function continuesQuoteRun(
  previous: QuoteLineAnchor,
  anchor: QuoteLineAnchor,
): boolean {
  if (previous.origin !== anchor.origin) return false;
  if (previous.origin === "diff") return true;
  return (
    anchor.path === previous.path &&
    anchor.lineNumber === previous.lineNumber + 1
  );
}

/**
 * Groups range-collected anchors into chips. File-preview gaps still split so
 * skipped lines do not look contiguous. A Diff drag stays one chip across
 * add/delete and collapsed hunks; add vs delete lives in the unified snippet.
 */
export function buildQuoteSelections<T extends QuoteLineAnchor>(
  anchors: readonly T[],
): QuoteLineSelection[] {
  const selections: QuoteLineSelection[] = [];
  let run: T[] = [];
  const flush = () => {
    if (run.length === 0) return;
    selections.push(selectionFromAnchors(run));
    run = [];
  };
  for (const anchor of anchors) {
    const previous = run[run.length - 1];
    if (previous !== undefined && !continuesQuoteRun(previous, anchor)) {
      flush();
    }
    run.push(anchor);
  }
  flush();
  return selections;
}

/**
 * Resolves the quote row key under the pointer, restricted to `root`.
 * `group` is the split-view side lock; `"any"` accepts every keyed cell so a
 * unified drag can leave a delete row onto the insert below it. Falls back
 * from a code cell to the row's keyed gutter so dragging works across the
 * whole line, not only the narrow gutter.
 */
export function quoteKeyFromPoint(
  clientX: number,
  clientY: number,
  root: HTMLElement,
  group: string | "any",
): string | null {
  const element = document.elementFromPoint(clientX, clientY);
  if (!(element instanceof Element) || !root.contains(element)) return null;
  const anyGroup = group === "any";
  const direct = element.closest<HTMLElement>("[data-quote-key]");
  if (direct !== null && (anyGroup || direct.dataset.quoteGroup === group)) {
    return direct.dataset.quoteKey ?? null;
  }
  const row = element.closest("tr");
  const selector = anyGroup
    ? "[data-quote-key]"
    : `[data-quote-key][data-quote-group="${group}"]`;
  return row?.querySelector<HTMLElement>(selector)?.dataset.quoteKey ?? null;
}

/**
 * File preview: the keyed row itself. Diff: that side's gutter <td> and the
 * following code <td> — never the whole <tr>, because split view shares one
 * row across old and new.
 */
export function quotePaintTargets(el: HTMLElement): HTMLElement[] {
  const td = el.closest("td");
  if (td === null) return [el];
  const targets = [td];
  let sibling = td.nextElementSibling;
  while (sibling instanceof HTMLElement && sibling.matches("td")) {
    if (isDiffGutterCell(sibling)) {
      // Unified: old/new number columns sit next to each other; keep walking
      // to the code cell. Split: the next cell is already code, so this arm
      // does not run.
      targets.push(sibling);
      sibling = sibling.nextElementSibling;
      continue;
    }
    targets.push(sibling);
    break;
  }
  return targets;
}

function isDiffGutterCell(td: HTMLElement): boolean {
  return (
    td.classList.contains("diff-gutter") ||
    td.querySelector(
      "[data-quote-key], [data-quote-gutter], .ora-diff-quote-gutter",
    ) !== null
  );
}

function markQuoteAttr(
  els: readonly HTMLElement[],
  attr: "data-quote-selected" | "data-quote-pinned",
  on: boolean,
): void {
  for (const el of els) {
    if (on) el.setAttribute(attr, "true");
    else el.removeAttribute(attr);
  }
}

function clearPinnedAttributes(root: HTMLElement | null): void {
  root
    ?.querySelectorAll("[data-quote-pinned]")
    .forEach((el) => el.removeAttribute("data-quote-pinned"));
}
