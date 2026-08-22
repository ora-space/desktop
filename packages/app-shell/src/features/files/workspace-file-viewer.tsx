import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import { ScrollArea } from "@ora/ui";
import type { BundledLanguage, ThemedTokenWithVariants } from "shiki";
import { useTranslation } from "react-i18next";
import { workspaceFileVisual } from "./workspace-file-visuals";
import { utf8ByteColumnToStringIndex } from "./workspace-file-viewer-utils";
import {
  useQuoteLineSelection,
  type QuoteLineAnchor,
} from "../quote-line-selection";
import "./workspace-file-viewer.css";
import "../quote-line-selection.css";

const MAX_HIGHLIGHT_BYTES = 512 * 1024;

export interface WorkspaceFileMatchTarget {
  line: number;
  column: number;
  matchedText: string;
}

interface WorkspaceFileViewerProps {
  content: string;
  path: string;
  target: WorkspaceFileMatchTarget | null;
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
}: WorkspaceFileViewerProps) {
  const { t } = useTranslation();
  const targetRow = useRef<HTMLSpanElement | null>(null);
  const lines = useMemo(() => content.split(/\r\n|\n|\r/), [content]);
  const language = workspaceFileVisual(path).language;
  const contentByteLength = useMemo(
    () => new TextEncoder().encode(content).byteLength,
    [content],
  );
  const highlightEnabled = contentByteLength <= MAX_HIGHLIGHT_BYTES;
  const highlightKey = highlightEnabled ? `${language}\u0000${content}` : null;
  const [highlighted, setHighlighted] = useState<HighlightedFile | null>(null);

  const anchors = useMemo<QuoteLineAnchor[]>(
    () =>
      lines.map((line, index) => ({
        key: String(index + 1),
        lineNumber: index + 1,
        snippet: line,
        path,
      })),
    [lines, path],
  );

  const {
    rootRef,
    onGutterMouseDown,
    onPlusMouseDown,
    onPlusClick,
    onNumberClick,
    onNumberKeyDown,
  } = useQuoteLineSelection({ anchors });

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
    targetRow.current?.scrollIntoView({ block: "center", inline: "nearest" });
  }, [content, target]);

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {!highlightEnabled && (
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
          }}
          data-quote-root
          data-selectable
          className="workspace-file-viewer min-w-max py-4 font-mono text-xs leading-5 text-foreground"
        >
          <code>
            {lines.map((line, index) => {
              const lineNumber = index + 1;
              const isTarget = target?.line === lineNumber;
              const match = isTarget
                ? matchRange(line, target.column, target.matchedText)
                : null;
              return (
                <span
                  key={lineNumber}
                  ref={isTarget ? targetRow : undefined}
                  aria-current={isTarget ? "location" : undefined}
                  data-line-number={lineNumber}
                  data-quote-key={lineNumber}
                  className={`workspace-file-line group/line relative block ${isTarget ? "bg-amber-500/10" : ""}`}
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
                      onClick={(event) =>
                        onNumberClick(event, String(lineNumber))
                      }
                      onKeyDown={(event) =>
                        onNumberKeyDown(event, String(lineNumber))
                      }
                    >
                      {lineNumber}
                    </span>
                    <button
                      type="button"
                      tabIndex={-1}
                      data-quote-button
                      className="workspace-file-quote-btn"
                      aria-label={t("files.quoteLineToChat", {
                        line: lineNumber,
                      })}
                      onMouseDown={(event) =>
                        onPlusMouseDown(event, String(lineNumber))
                      }
                      onClick={(event) =>
                        onPlusClick(event, String(lineNumber))
                      }
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
            })}
          </code>
        </pre>
      </ScrollArea>
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
