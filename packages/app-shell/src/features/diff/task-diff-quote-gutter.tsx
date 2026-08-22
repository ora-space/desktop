import { useMemo, type ReactNode } from "react";
import {
  getChangeKey,
  type FileData,
  type GutterOptions,
} from "react-diff-view";
import { useTranslation } from "react-i18next";
import {
  useQuoteLineSelection,
  type QuoteLineAnchor,
} from "../quote-line-selection";
import {
  diffQuoteAnchorFor,
  unifiedDiffQuoteLine,
  type DiffQuoteAnchor,
} from "./task-diff-quote";
import "../quote-line-selection.css";

export type DiffQuoteViewType = "unified" | "split";

/**
 * Adapts the shared quote-line selection to react-diff-view gutters. Anchors
 * carry `${side}:${line}` keys and a `group` per side, so a drag started on
 * the old column of a split view can never leak into the new column (or vice
 * versa), and delete rows still quote from the old path.
 *
 * Unified view hosts every `+` in the old (left) gutter so delete/insert/
 * context share one vertical track; the quote key still follows the side
 * that owns the line. Unified drags are not locked to one side, so a press
 * on a delete can continue onto inserts below.
 */
export function useTaskDiffQuoteGutter(
  file: FileData,
  viewType: DiffQuoteViewType = "split",
) {
  const { t } = useTranslation();

  const anchors = useMemo<QuoteLineAnchor[]>(() => {
    const collected: QuoteLineAnchor[] = [];
    for (const anchor of collectFileQuoteAnchors(file)) {
      collected.push({
        key: `${anchor.side}:${anchor.line}`,
        lineNumber: anchor.line,
        snippet: unifiedDiffQuoteLine(anchor.changeType, anchor.content),
        path: anchor.path,
        group: anchor.side,
        origin: "diff",
        diffSide: anchor.side,
      });
    }
    return collected;
  }, [file]);

  const {
    rootRef,
    onGutterMouseDown,
    onPlusMouseDown,
    onPlusClick,
    onNumberClick,
    onNumberKeyDown,
  } = useQuoteLineSelection({
    anchors,
    enabled: anchors.length > 0,
    lockToGroup: viewType === "split",
  });

  const renderGutter = useMemo(() => {
    const quoteCell = (
      anchor: DiffQuoteAnchor,
      options: {
        wrapInAnchor: GutterOptions["wrapInAnchor"];
        inHoverState: boolean;
        showPlus: boolean;
        number: ReactNode;
      },
    ) => {
      const key = `${anchor.side}:${anchor.line}`;
      return options.wrapInAnchor(
        <span
          className={
            options.showPlus
              ? "ora-diff-quote-gutter"
              : "ora-diff-quote-gutter ora-diff-quote-gutter--plain"
          }
          data-quote-key={key}
          data-quote-group={anchor.side}
          data-quote-gutter
          data-quote-hover={options.inHoverState ? "true" : undefined}
          data-diff-quote-key={anchor.changeKey}
          onMouseDown={(event) => onGutterMouseDown(event, key)}
        >
          {options.showPlus ? (
            <button
              type="button"
              tabIndex={-1}
              className="ora-diff-quote-plus"
              data-quote-button
              aria-label={t("diff.quoteLineToChat", { line: anchor.line })}
              onMouseDown={(event) => onPlusMouseDown(event, key)}
              onClick={(event) => onPlusClick(event, key)}
            />
          ) : null}
          <span
            className="ora-diff-quote-number"
            data-quote-number
            {...(options.number == null
              ? { "aria-hidden": true as const }
              : {
                  role: "button" as const,
                  tabIndex: 0,
                  "aria-label": t("diff.selectLine", { line: anchor.line }),
                  // The + is deliberately not tabbable, so the number carries
                  // the keyboard route to the same quote action.
                  "aria-keyshortcuts": "Control+Enter Meta+Enter",
                })}
            onClick={(event) => onNumberClick(event, key)}
            onKeyDown={(event) => onNumberKeyDown(event, key)}
          >
            {options.number}
          </span>
        </span>,
      );
    };

    const plainCell = (
      wrapInAnchor: GutterOptions["wrapInAnchor"],
      number: ReactNode,
    ) =>
      wrapInAnchor(
        <span className="ora-diff-quote-gutter ora-diff-quote-gutter--plain">
          <span className="ora-diff-quote-number">{number}</span>
        </span>,
      );

    return ({
      change,
      side,
      renderDefault,
      wrapInAnchor,
      inHoverState,
    }: GutterOptions) => {
      const changeKey = getChangeKey(change);

      if (viewType === "unified") {
        if (side === "old") {
          if (change.type === "delete") {
            const anchor = diffQuoteAnchorFor(file, change, "old", changeKey);
            if (anchor === null)
              return plainCell(wrapInAnchor, renderDefault());
            return quoteCell(anchor, {
              wrapInAnchor,
              inHoverState,
              showPlus: true,
              number: renderDefault(),
            });
          }
          // Inserts and context: number lives in the new column; the + stays
          // in this left column so it lines up with delete rows.
          const anchor = diffQuoteAnchorFor(file, change, "new", changeKey);
          if (anchor === null) return plainCell(wrapInAnchor, renderDefault());
          return quoteCell(anchor, {
            wrapInAnchor,
            inHoverState,
            showPlus: true,
            number: null,
          });
        }
        if (change.type === "delete") {
          return plainCell(wrapInAnchor, renderDefault());
        }
        const anchor = diffQuoteAnchorFor(file, change, "new", changeKey);
        if (anchor === null) return plainCell(wrapInAnchor, renderDefault());
        return quoteCell(anchor, {
          wrapInAnchor,
          inHoverState,
          showPlus: false,
          number: renderDefault(),
        });
      }

      // Split view keeps both columns numbered: context rows are quoted from
      // the new side only, but the old column still shows its own line number.
      // Collapsing context rows to a single number is a unified-view concern.
      const anchor = diffQuoteAnchorFor(file, change, side, changeKey);
      if (anchor === null) {
        return plainCell(wrapInAnchor, renderDefault());
      }
      return quoteCell(anchor, {
        wrapInAnchor,
        inHoverState,
        showPlus: true,
        number: renderDefault(),
      });
    };
  }, [
    file,
    onGutterMouseDown,
    onNumberClick,
    onNumberKeyDown,
    onPlusClick,
    onPlusMouseDown,
    t,
    viewType,
  ]);

  return { renderGutter, quoteRootRef: rootRef };
}

/** Precomputes every quoteable gutter cell so drag ranges resolve in O(1). */
function collectFileQuoteAnchors(file: FileData): DiffQuoteAnchor[] {
  const anchors: DiffQuoteAnchor[] = [];
  for (const hunk of file.hunks) {
    for (const change of hunk.changes) {
      const changeKey = getChangeKey(change);
      for (const side of ["old", "new"] as const) {
        const anchor = diffQuoteAnchorFor(file, change, side, changeKey);
        if (anchor !== null) anchors.push(anchor);
      }
    }
  }
  return anchors;
}
