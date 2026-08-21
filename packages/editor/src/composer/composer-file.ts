import { Node, mergeAttributes, type JSONContent } from "@tiptap/core";

export interface ComposerFileAttrs {
  path: string;
  startLine?: number;
  endLine?: number;
  /** When `directory`, the chip renders a folder glyph; payload stays a path. */
  kind?: "file" | "directory";
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
 * Visible label for a file chip: basename plus an optional line range.
 */
export function composerFileLabel(attrs: ComposerFileAttrs): string {
  const name = fileName(attrs.path);
  const range = lineRange(attrs);
  return range === null ? name : `${name}:${range}`;
}

/**
 * Wire format matching the previous backtick path:line payload the agent sees.
 */
export function composerFilePlainText(attrs: ComposerFileAttrs): string {
  const range = lineRange(attrs);
  const target = range === null ? attrs.path : `${attrs.path}:${range}`;
  return `\`${target}\``;
}

/**
 * Normalizes chip attrs from TipTap/DOM so NaN line numbers never reach the
 * agent payload as `:NaN`.
 */
export function composerFileAttrsFromUnknown(attrs: {
  path?: unknown;
  startLine?: unknown;
  endLine?: unknown;
  kind?: unknown;
}): ComposerFileAttrs {
  return {
    path: String(attrs.path ?? ""),
    startLine: optionalLineNumber(attrs.startLine),
    endLine: optionalLineNumber(attrs.endLine),
    kind: attrs.kind === "directory" ? "directory" : "file",
  };
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
      kind: file.kind ?? "file",
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
  draggable: true,

  addAttributes() {
    return {
      path: { default: "" },
      startLine: { default: null },
      endLine: { default: null },
      kind: { default: "file" },
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
          };
        },
      },
    ];
  },

  renderHTML({ node, HTMLAttributes }) {
    const attrs = composerFileAttrsFromUnknown(node.attrs);
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
        title: composerFilePlainText(attrs).replace(/`/g, ""),
      }),
      ["span", { class: "composer-chip-glyph", "aria-hidden": "true" }],
      ["span", { class: "composer-chip-label" }, composerFileLabel(attrs)],
    ];
  },

  renderText({ node }) {
    return composerFilePlainText(composerFileAttrsFromUnknown(node.attrs));
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
              return editor
                .chain()
                .deleteRange({ from: $from.pos - 1, to: $from.pos })
                .insertContent(content)
                .run();
            }
          }
          return commands.insertContent(content);
        },
    };
  },
});
