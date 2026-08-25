import { Node, mergeAttributes, type JSONContent } from "@tiptap/core";

export interface ComposerFileAttrs {
  path: string;
  startLine?: number;
  endLine?: number;
  /**
   * Source text captured at quote time. File-preview quotes expand to a
   * `start:end:path` citation fence. Diff quotes store unified `+/-/ ` lines
   * and expand to a mini `diff --git` patch so the agent sees an existing
   * git change (add vs delete), not current file contents.
   */
  snippet?: string;
  /** When `directory`, the chip renders a folder glyph; payload stays a path. */
  kind?: "file" | "directory";
  /**
   * Diff-gutter quotes. File explorer / @ mentions omit this so send stays a
   * source citation. Send expands to a `diff --git` patch the agent can treat
   * as a review comment on that change.
   */
  origin?: "diff";
  /**
   * GitHub-style patch side: `old` is deletes, `new` is inserts and context.
   * Omitted when one chip spans both sides; the snippet still has `+/-/ `.
   */
  diffSide?: "old" | "new";
}

declare module "@tiptap/core" {
  interface Commands<ReturnType> {
    composerFile: {
      /** Inserts workspace file chips without round-tripping through plain text. */
      insertComposerFiles: (files: ComposerFileAttrs[]) => ReturnType;
    };
  }
}

/**
 * Visible basename for a file chip (line range is rendered separately in the app shell).
 */
export function composerFileLabel(attrs: ComposerFileAttrs): string {
  return fileName(attrs.path);
}

/**
 * Cursor-style line range label (`L12` or `L12-34`), or null when the chip is path-only.
 */
export function composerFileLineRangeLabel(
  attrs: ComposerFileAttrs,
): string | null {
  const range = lineRange(attrs);
  return range === null ? null : `L${range}`;
}

/**
 * Agent payload: diff-gutter quotes become a mini `diff --git` patch
 * (add/delete visible); file-preview quotes stay a `start:end:path` citation;
 * path-only `@` mentions stay a backtick path.
 */
export function composerFilePlainText(attrs: ComposerFileAttrs): string {
  if (attrs.snippet !== undefined && attrs.startLine !== undefined) {
    const end = attrs.endLine ?? attrs.startLine;
    const fence = codeFenceMarker(attrs.snippet);
    if (attrs.origin === "diff") {
      return diffQuotePlainText(
        attrs.path,
        attrs.startLine,
        end,
        attrs.snippet,
        attrs.diffSide,
        fence,
      );
    }
    return `\n${fence}${attrs.startLine}:${end}:${attrs.path}\n${attrs.snippet}\n${fence}\n`;
  }
  const range = lineRange(attrs);
  const target = range === null ? attrs.path : `${attrs.path}:${range}`;
  return `\`${target}\``;
}

/**
 * Hover title for the chip. Multiline payloads (citation fences / diff patches)
 * use the path so the native tooltip stays one line; ranged `@` mentions keep
 * `path:start-end` without wrapping backticks.
 */
export function composerFileChipTitle(attrs: ComposerFileAttrs): string {
  const payload = composerFilePlainText(attrs).replace(/`/g, "");
  return payload.includes("\n") ? attrs.path : payload;
}

/**
 * Normalizes chip attrs from TipTap/DOM so NaN line numbers never reach the
 * agent payload as `:NaN`. Also maps JSON `null` and quote metadata.
 */
export function composerFileAttrsFromUnknown(attrs: {
  path?: unknown;
  startLine?: unknown;
  endLine?: unknown;
  snippet?: unknown;
  kind?: unknown;
  origin?: unknown;
  diffSide?: unknown;
}): ComposerFileAttrs {
  const origin = attrs.origin;
  const diffSide = attrs.diffSide;
  return {
    path: String(attrs.path ?? ""),
    startLine: optionalLineNumber(attrs.startLine),
    endLine: optionalLineNumber(attrs.endLine),
    snippet: optionalString(attrs.snippet),
    kind: attrs.kind === "directory" ? "directory" : "file",
    origin: origin === "diff" ? "diff" : undefined,
    diffSide: diffSide === "old" || diffSide === "new" ? diffSide : undefined,
  };
}

/** Reads chip attrs from a TipTap/ProseMirror node through the same normalizer. */
export function composerFileAttrsFromNode(node: {
  attrs: Record<string, unknown>;
}): ComposerFileAttrs {
  return composerFileAttrsFromUnknown(node.attrs);
}

function optionalString(value: unknown): string | undefined {
  if (value === null || value === undefined) return undefined;
  return String(value);
}

/**
 * Mini `git diff` patch. Agents are trained on `diff --git` / `--- a/` / `+++ b/`
 * far more than a custom fence info string, so this reads as an existing change
 * the user is commenting on — not a file citation and not "apply this hunk".
 */
function diffQuotePlainText(
  path: string,
  startLine: number,
  endLine: number,
  snippet: string,
  diffSide: "old" | "new" | undefined,
  fence: string,
): string {
  const { oldCount, newCount } = unifiedHunkCounts(snippet);
  const side =
    diffSide === "old" || diffSide === "new" ? ` (${diffSide} side)` : "";
  // The hunk counts describe the quoted lines, not the span they came from: a
  // drag across a collapsed hunk quotes lines 2 and 40 as a two-line body. The
  // range note keeps the real span for the agent, and lets chat history rebuild
  // the chip label from the payload alone.
  const note = ` quoted from git diff${side}, lines ${startLine}-${endLine}`;
  const hunk = `@@ -${startLine},${oldCount} +${startLine},${newCount} @@${note}`;
  const body = [
    `diff --git a/${path} b/${path}`,
    `--- a/${path}`,
    `+++ b/${path}`,
    hunk,
    snippet,
  ].join("\n");
  return `\n${fence}diff\n${body}\n${fence}\n`;
}

/** Counts old/new lines from unified `+/-/ ` prefixes so the @@ header matches the body. */
function unifiedHunkCounts(snippet: string): {
  oldCount: number;
  newCount: number;
} {
  let oldCount = 0;
  let newCount = 0;
  for (const line of snippet.split("\n")) {
    if (line.startsWith("+")) {
      newCount += 1;
    } else if (line.startsWith("-")) {
      oldCount += 1;
    } else {
      oldCount += 1;
      newCount += 1;
    }
  }
  return { oldCount, newCount };
}

function fileName(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, "");
  const parts = trimmed.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

function optionalLineNumber(value: unknown): number | undefined {
  if (value === null || value === undefined || value === "") {
    return undefined;
  }
  const parsed = typeof value === "number" ? value : Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return undefined;
  }
  return parsed;
}

function lineRange(attrs: ComposerFileAttrs): string | null {
  if (attrs.startLine === undefined) {
    return null;
  }
  if (attrs.endLine === undefined || attrs.endLine === attrs.startLine) {
    return String(attrs.startLine);
  }
  return `${attrs.startLine}-${attrs.endLine}`;
}

/** Picks a fence long enough that the snippet cannot close it early. */
function codeFenceMarker(snippet: string): string {
  let ticks = 3;
  const longest = snippet.match(/`+/g)?.reduce((max, run) => {
    return Math.max(max, run.length);
  }, 0);
  if (longest !== undefined && longest >= ticks) {
    ticks = longest + 1;
  }
  return "`".repeat(ticks);
}

function fileContent(files: ComposerFileAttrs[]): JSONContent[] {
  // No text spaces between chips: native selection paints those spaces as
  // caret-thin blue bars. Visual gap comes from chip margin; plain-text
  // serialization inserts spaces between adjacent chips for the agent payload.
  const chips: JSONContent[] = files.map((file) => ({
    type: "composerFile",
    attrs: {
      path: file.path,
      startLine: file.startLine ?? null,
      endLine: file.endLine ?? null,
      snippet: file.snippet ?? null,
      kind: file.kind ?? "file",
      origin: file.origin ?? null,
      diffSide: file.diffSide ?? null,
    },
  }));
  return [...chips, { type: "text", text: " " }];
}

/**
 * Inline file-range chip for explorer selections. Atom so typing after it
 * stays body text, same exclusive model as link chips.
 */
export const ComposerFile = Node.create({
  name: "composerFile",
  group: "inline",
  inline: true,
  atom: true,
  selectable: true,
  // False so a mousedown-move is a text selection, not an HTML5 node drag.
  draggable: false,

  addAttributes() {
    return {
      path: { default: "" },
      startLine: { default: null },
      endLine: { default: null },
      snippet: { default: null },
      kind: { default: "file" },
      origin: { default: null },
      diffSide: { default: null },
    };
  },

  parseHTML() {
    return [
      {
        tag: "span[data-composer-file]",
        getAttrs: (element) => {
          if (!(element instanceof HTMLElement)) return false;
          const kindAttr = element.getAttribute("data-kind");
          const kind = kindAttr === "directory" ? "directory" : "file";
          const startLine = element.getAttribute("data-start-line");
          const endLine = element.getAttribute("data-end-line");
          return {
            path: element.getAttribute("data-composer-file") ?? "",
            kind,
            startLine: optionalLineNumber(startLine) ?? null,
            endLine: optionalLineNumber(endLine) ?? null,
            // Snippets are TipTap-JSON only; HTML restore cannot carry bodies.
            snippet: null,
            origin: null,
            diffSide: null,
          };
        },
      },
    ];
  },

  renderHTML({ node, HTMLAttributes }) {
    const attrs = composerFileAttrsFromUnknown(node.attrs);
    const rangeLabel = composerFileLineRangeLabel(attrs);
    return [
      "span",
      mergeAttributes(HTMLAttributes, {
        "data-composer-file": attrs.path,
        "data-kind": attrs.kind ?? "file",
        ...(attrs.startLine === undefined
          ? {}
          : { "data-start-line": String(attrs.startLine) }),
        ...(attrs.endLine === undefined
          ? {}
          : { "data-end-line": String(attrs.endLine) }),
        class: "composer-chip composer-chip-file",
        contenteditable: "false",
        title: composerFileChipTitle(attrs),
      }),
      ["span", { class: "composer-chip-glyph", "aria-hidden": "true" }],
      [
        "span",
        { class: "composer-chip-label" },
        rangeLabel === null
          ? composerFileLabel(attrs)
          : `${composerFileLabel(attrs)} ${rangeLabel}`,
      ],
    ];
  },

  renderText({ node }) {
    return composerFilePlainText(composerFileAttrsFromNode(node));
  },

  addCommands() {
    return {
      insertComposerFiles:
        (files) =>
        ({ editor, commands, state }) => {
          if (files.length === 0) {
            return false;
          }
          const content = fileContent(files);
          if (editor.isEmpty) {
            return commands.setContent({
              type: "doc",
              content: [{ type: "paragraph", content }],
            });
          }
          // Drop one separator space after a prior chip/token so a range
          // selection does not paint a blue bar between adjacent atoms.
          const { $from } = state.selection;
          const index = $from.index();
          if (index >= 2) {
            const maybeSpace = $from.parent.child(index - 1);
            const maybeAtom = $from.parent.child(index - 2);
            if (
              maybeSpace.isText &&
              maybeSpace.text === " " &&
              (maybeAtom.type.name === "composerFile" ||
                maybeAtom.type.name === "promptToken")
            ) {
              // Both steps go through `commands`, which share this command's
              // own transaction. A nested `editor.chain().run()` would dispatch
              // a second transaction while this one is still open, so the outer
              // (now stale) transaction is applied to a state it was not built
              // from and ProseMirror throws "Applying a mismatched
              // transaction" after the chips have already landed.
              return (
                commands.deleteRange({ from: $from.pos - 1, to: $from.pos }) &&
                commands.insertContent(content)
              );
            }
          }
          return commands.insertContent(content);
        },
    };
  },
});
