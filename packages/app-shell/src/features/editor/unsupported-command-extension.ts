import { Extension } from "@tiptap/core";
import type { EditorState } from "@tiptap/pm/state";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";

interface UnsupportedCommandExtensionOptions {
  /** Reads the current Agent command catalog without rebuilding the editor. */
  availableCommandNames: () => ReadonlySet<string> | undefined;
  /** Reads the localized explanation used by the native browser tooltip. */
  commandTitle: (commandName: string) => string | undefined;
}

/** Adds a compact warning marker to an unknown leading slash command. */
export const UnsupportedCommandExtension =
  Extension.create<UnsupportedCommandExtensionOptions>({
    name: "unsupportedCommand",

    addOptions() {
      return {
        availableCommandNames: () => undefined,
        commandTitle: () => undefined,
      };
    },

    addProseMirrorPlugins() {
      return [
        new Plugin({
          key: new PluginKey("unsupportedCommand"),
          props: {
            decorations: (state) => {
              const availableCommandNames =
                this.options.availableCommandNames();
              if (availableCommandNames === undefined)
                return DecorationSet.empty;
              const command = leadingCommand(state);
              if (command === null || availableCommandNames.has(command.name)) {
                return DecorationSet.empty;
              }
              return DecorationSet.create(state.doc, [
                Decoration.inline(command.from, command.to, {
                  class: "composer-unsupported-command",
                  title: this.options.commandTitle(command.name),
                  "data-unsupported-command": command.name,
                }),
              ]);
            },
          },
        }),
      ];
    },
  });

function leadingCommand(
  state: EditorState,
): { name: string; from: number; to: number } | null {
  const firstBlock = state.doc.firstChild;
  const firstInline = firstBlock?.firstChild;
  if (
    firstBlock?.type.name !== "paragraph" ||
    firstInline === null ||
    firstInline === undefined ||
    !firstInline.isText ||
    firstInline.text === undefined ||
    firstInline.marks.length > 0
  ) {
    return null;
  }
  const match = firstInline.text.match(/^\s*\/([A-Za-z][\w-]*)(?=\s|$)/);
  const name = match?.[1];
  if (match === null || name === undefined) return null;
  const slashOffset = match[0].indexOf("/");
  return {
    name,
    from: 1 + slashOffset,
    to: 1 + match[0].length,
  };
}
