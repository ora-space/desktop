import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from "react";
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuTrigger,
  ScrollArea,
} from "@ora/ui";
import type { BundledLanguage, ThemedTokenWithVariants } from "shiki";
import { useTranslation } from "react-i18next";
import { IconMessagePlus } from "@tabler/icons-react";
import { workspaceFileVisual } from "./workspace-file-visuals";
import { utf8ByteColumnToStringIndex } from "./workspace-file-viewer-utils";

const MAX_HIGHLIGHT_BYTES = 512 * 1024;

export interface WorkspaceFileMatchTarget {
  line: number;
  column: number;
  matchedText: string;
}

export interface WorkspaceFileLineSelection {
  path: string;
  startLine: number;
  endLine: number;
}

interface WorkspaceFileViewerProps {
  content: string;
  path: string;
  target: WorkspaceFileMatchTarget | null;
  onAddLineSelectionToChat?: (selection: WorkspaceFileLineSelection) => void;
}

interface ShikiTokenStyle extends CSSProperties {
  "--shiki-dark"?: string;
}

const highlightedFileCache = new Map<string, Promise<ThemedTokenWithVariants[][] | null>>();

interface HighlightedFile {
  key: string;
  tokens: ThemedTokenWithVariants[][] | null;
}

/** Renders UTF-8 text with stable line numbers and scrolls selected search matches into view. */
export function WorkspaceFileViewer({ content, path, target, onAddLineSelectionToChat }: WorkspaceFileViewerProps) {
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
  const [lineSelection, setLineSelection] = useState<{ startLine: number; endLine: number } | null>(null);
  const lineSelectionRef = useRef<{ startLine: number; endLine: number } | null>(null);
  const lineDragAnchorRef = useRef<number | null>(null);
  const lineDraggingRef = useRef(false);
  const lineDraggedRef = useRef(false);

  useEffect(() => {
    const stopLineDrag = () => {
      lineDraggingRef.current = false;
      lineDragAnchorRef.current = null;
    };
    window.addEventListener("mouseup", stopLineDrag);
    return () => window.removeEventListener("mouseup", stopLineDrag);
  }, []);

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
        .then(({ codeToTokensWithThemes }) => codeToTokensWithThemes(content, {
          lang: language as BundledLanguage,
          themes: { light: "light-plus", dark: "dark-plus" },
        }))
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

  /** Captures the selected browser text and anchors the context menu to its line range. */
  const handleContextMenu = (event: React.MouseEvent<HTMLPreElement>) => {
    const target = event.target instanceof Element
      ? event.target.closest<HTMLElement>("[data-line-number]")
      : null;
    const rawLine = target?.dataset.lineNumber;
    const lineNumber = rawLine === undefined ? null : Number(rawLine);
    if (lineNumber === null || !Number.isInteger(lineNumber)) return;

    const browserSelection = lineSelectionFromBrowserSelection(lineNumber);
    if (browserSelection === null) return;
    const existing = lineSelectionRef.current;
    const selected = browserSelection.startLine === lineNumber
      && browserSelection.endLine === lineNumber
      && existing !== null
      && lineNumber >= existing.startLine
      && lineNumber <= existing.endLine
      ? existing
      : browserSelection;
    lineSelectionRef.current = selected;
    setLineSelection(selected);
  };

  /** Mirrors a left-button drag over line numbers into a contiguous line range. */
  const updateDraggedLineSelection = (lineNumber: number) => {
    const anchor = lineDragAnchorRef.current;
    if (!lineDraggingRef.current || anchor === null) return;
    const selected = {
      startLine: Math.min(anchor, lineNumber),
      endLine: Math.max(anchor, lineNumber),
    };
    lineDraggedRef.current = lineNumber !== anchor;
    lineSelectionRef.current = selected;
    setLineSelection(selected);
  };

  /** Captures a native left-button text selection so its row range is visible before the menu opens. */
  const handleTextSelectionMouseUp = () => {
    const selected = lineSelectionFromBrowserSelection();
    if (selected === null) return;
    lineSelectionRef.current = selected;
    setLineSelection(selected);
  };

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
      <ContextMenu>
        <ContextMenuTrigger
          render={(
            <pre
              data-selectable
              className="workspace-file-viewer min-w-max py-4 font-mono text-xs leading-5 text-foreground"
              onContextMenu={handleContextMenu}
              onMouseUp={handleTextSelectionMouseUp}
            />
          )}
        >
          <code>
          {lines.map((line, index) => {
            const lineNumber = index + 1;
            const isTarget = target?.line === lineNumber;
            const match = isTarget
              ? matchRange(line, target.column, target.matchedText)
              : null;
            const isSelected = lineSelection !== null
              && lineNumber >= lineSelection.startLine
              && lineNumber <= lineSelection.endLine;
            const row = (
              <span
                key={lineNumber}
                ref={isTarget ? targetRow : undefined}
                aria-current={isTarget ? "location" : undefined}
                data-line-number={lineNumber}
                className={`block ${isTarget ? "bg-amber-500/10" : ""} ${isSelected ? "bg-sky-500/10" : ""}`}
              >
                <span
                  role="button"
                  tabIndex={0}
                  aria-label={t("files.selectLine", { line: lineNumber })}
                  className="sticky left-0 inline-block w-14 cursor-pointer select-none bg-background pr-3 text-right text-muted-foreground/65 hover:text-foreground"
                  onMouseDown={(event) => {
                    if (event.button !== 0) return;
                    event.preventDefault();
                    window.getSelection()?.removeAllRanges();
                    lineDragAnchorRef.current = lineNumber;
                    lineDraggingRef.current = true;
                    lineDraggedRef.current = false;
                    lineSelectionRef.current = { startLine: lineNumber, endLine: lineNumber };
                    setLineSelection({ startLine: lineNumber, endLine: lineNumber });
                  }}
                  onMouseEnter={() => updateDraggedLineSelection(lineNumber)}
                  onClick={(event) => {
                    if (lineDraggedRef.current) {
                      lineDraggedRef.current = false;
                      return;
                    }
                    const existing = lineSelectionRef.current;
                    const selected = event.shiftKey && existing !== null
                      ? {
                          startLine: Math.min(existing.startLine, lineNumber),
                          endLine: Math.max(existing.endLine, lineNumber),
                        }
                      : { startLine: lineNumber, endLine: lineNumber };
                    lineSelectionRef.current = selected;
                    setLineSelection(selected);
                  }}
                  onKeyDown={(event) => {
                    if (event.key !== "Enter" && event.key !== " ") return;
                    event.preventDefault();
                    const existing = lineSelectionRef.current;
                    const selected = event.shiftKey && existing !== null
                      ? {
                          startLine: Math.min(existing.startLine, lineNumber),
                          endLine: Math.max(existing.endLine, lineNumber),
                        }
                      : { startLine: lineNumber, endLine: lineNumber };
                    lineSelectionRef.current = selected;
                    setLineSelection(selected);
                  }}
                >
                  {lineNumber}
                </span>
                <span className="px-3">
                  {renderHighlightedLine(
                    line,
                    highlighted?.key === highlightKey ? highlighted.tokens?.[index] : undefined,
                    match,
                  )}
                </span>
              </span>
            );
            return row;
          })}
          </code>
        </ContextMenuTrigger>
        {onAddLineSelectionToChat !== undefined && (
          <ContextMenuContent>
            <ContextMenuItem
              onClick={() => {
                const selection = lineSelectionRef.current;
                if (selection === null) return;
                onAddLineSelectionToChat({ path, ...selection });
              }}
            >
              <IconMessagePlus />
              {t("files.addLineSelectionToChat", {
                startLine: lineSelection?.startLine ?? 1,
                endLine: lineSelection?.endLine ?? 1,
              })}
            </ContextMenuItem>
          </ContextMenuContent>
        )}
      </ContextMenu>
      </ScrollArea>
    </div>
  );
}

/** Resolves the selected browser text to the line range used by the AI context action. */
function lineSelectionFromBrowserSelection(
  fallbackLine?: number,
): { startLine: number; endLine: number } | null {
  const selection = window.getSelection();
  if (selection === null || selection.rangeCount === 0 || selection.isCollapsed) {
    return fallbackLine === undefined
      ? null
      : { startLine: fallbackLine, endLine: fallbackLine };
  }
  const range = selection.getRangeAt(0);
  const startLine = lineNumberFromNode(range.startContainer) ?? fallbackLine ?? null;
  const endLine = lineNumberFromNode(range.endContainer) ?? fallbackLine ?? null;
  if (startLine === null || endLine === null) return null;
  return {
    startLine: Math.min(startLine, endLine),
    endLine: Math.max(startLine, endLine),
  };
}

/** Finds the nearest rendered line marker for a text-selection endpoint. */
function lineNumberFromNode(node: Node): number | null {
  const element = node.nodeType === Node.ELEMENT_NODE
    ? node as Element
    : node.parentElement;
  const line = element?.closest<HTMLElement>("[data-line-number]")?.dataset.lineNumber;
  if (line === undefined) return null;
  const parsed = Number(line);
  return Number.isInteger(parsed) ? parsed : null;
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
  if (matchedText.length === 0 || !line.startsWith(matchedText, start)) return null;
  return { start, end: start + matchedText.length };
}
