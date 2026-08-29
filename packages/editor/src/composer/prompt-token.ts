import { Node, mergeAttributes } from "@tiptap/core";

export type PromptTokenKind = "skill" | "command" | "role";

declare module "@tiptap/core" {
  interface Commands<ReturnType> {
    promptToken: {
      /** Inserts a skill, slash-command, or role mention at the caret. */
      setPromptToken: (kind: PromptTokenKind, name: string) => ReturnType;
    };
  }
}

function tokenText(kind: PromptTokenKind, name: string): string {
  if (kind === "command") return `/${name}`;
  if (kind === "role") return `@${name}`;
  return `$${name}`;
}

/**
 * Inline mention for `$skill` / `/command` / `@role`. Atom so the token deletes
 * as one unit. Color comes from soft mint wash + forest ink in app-shell CSS
 * (Cursor-style), not `--primary` (Ora primary is grayscale).
 */
export const PromptToken = Node.create({
  name: "promptToken",
  group: "inline",
  inline: true,
  atom: true,
  selectable: true,

  addAttributes() {
    return {
      kind: { default: "skill" },
      name: { default: "" },
    };
  },

  parseHTML() {
    return [{ tag: "span[data-prompt-token]" }];
  },

  renderHTML({ node, HTMLAttributes }) {
    const kind = node.attrs.kind as PromptTokenKind;
    const name = String(node.attrs.name);
    return [
      "span",
      mergeAttributes(HTMLAttributes, {
        "data-prompt-token": kind,
        class: "composer-mention",
        contenteditable: "false",
      }),
      tokenText(kind, name),
    ];
  },

  renderText({ node }) {
    return tokenText(
      node.attrs.kind as PromptTokenKind,
      String(node.attrs.name),
    );
  },

  addCommands() {
    return {
      setPromptToken:
        (kind, name) =>
        ({ commands }) => {
          const chip = { type: this.name, attrs: { kind, name } };
          return commands.insertContent([chip, { type: "text", text: " " }]);
        },
    };
  },
});
