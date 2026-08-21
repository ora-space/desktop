import type { AnyExtension, Editor } from "@tiptap/core";
import type { Node } from "@tiptap/pm/model";
import { TaskList } from "@tiptap/extension-list";
import { Placeholder } from "@tiptap/extensions";
import StarterKit from "@tiptap/starter-kit";
import { HorizontalRule } from "../node/horizontal-rule-node/horizontal-rule-node-extension";
import { ComposerCodeFence } from "./composer-code-fence";
import { ComposerChipSelection } from "./composer-chip-selection";
import { ComposerFile } from "./composer-file";
import { ComposerHighlight } from "./composer-highlight";
import { ComposerLink } from "./composer-link";
import {
  ComposerBold,
  ComposerCode,
  ComposerItalic,
  ComposerStrike,
  ComposerUnderline,
} from "./composer-marks";
import { ComposerMarkStartTyping } from "./composer-mark-start";
import { ComposerMarkdownBackfill } from "./composer-markdown-backfill";
import { ComposerMarkdownPaste } from "./composer-markdown";
import { ComposerMarkdownRevert } from "./composer-markdown-revert";
import { ComposerNewline } from "./composer-newline";
import { ComposerTaskItem } from "./composer-task-item";
import { PromptToken } from "./prompt-token";

export interface ComposerPlaceholderProps {
  editor: Editor;
  node: Node;
  pos: number;
  hasAnchor: boolean;
}

export const COMPOSER_FEATURE_SLOTS = [
  "codeBlock",
  "taskList",
  "highlight",
  "link",
  "fileChip",
  "promptToken",
] as const;

export type ComposerFeatureSlot = (typeof COMPOSER_FEATURE_SLOTS)[number];

/** Markdown ATX headings `#`–`######`. */
export const COMPOSER_HEADING_LEVELS = [1, 2, 3, 4, 5, 6] as const;

/**
 * Prompt-box capability minimum set. Full-page kit nodes (images, TOC, video,
 * alignment) stay out; product chrome can replace a slot instead of forking
 * the preset. `underline` is Mod-u only — `documentPlainText` cannot encode it,
 * so session switches must park TipTap JSON to keep the mark.
 */
export const COMPOSER_CAPABILITIES = {
  blocks: [
    "paragraph",
    "heading",
    "blockquote",
    "codeBlock",
    "bulletList",
    "orderedList",
    "taskList",
    "horizontalRule",
  ],
  headingLevels: COMPOSER_HEADING_LEVELS,
  marks: ["bold", "italic", "underline", "strike", "code", "highlight", "link"],
  chips: ["composerFile", "promptToken"],
} as const;

export interface ComposerExtensionOptions {
  /**
   * Placeholder shown in an empty composer. A function is re-read on each
   * decoration pass so callers can keep a stable extension list while copy changes.
   */
  placeholder?: string | ((props: ComposerPlaceholderProps) => string);
  /**
   * Omit a slot with `false` or swap in another extension. Unlisted slots keep
   * the default implementation.
   */
  features?: Partial<Record<ComposerFeatureSlot, AnyExtension | false>>;
  extraExtensions?: AnyExtension[];
}

/**
 * Resolves placeholder copy. The composer hint only belongs on a truly empty
 * editor, not on an empty heading after `# ` has already been converted.
 */
function placeholderText(
  options: ComposerExtensionOptions,
  props: ComposerPlaceholderProps,
): string {
  if (!props.editor.isEmpty || props.node.type.name !== "paragraph") {
    return "";
  }
  const { placeholder } = options;
  if (typeof placeholder === "function") {
    return placeholder(props);
  }
  return placeholder ?? "";
}

function resolveSlot(
  features: ComposerExtensionOptions["features"],
  slot: ComposerFeatureSlot,
  fallback: AnyExtension,
): AnyExtension | null {
  const override = features?.[slot];
  if (override === false) {
    return null;
  }
  return override ?? fallback;
}

function isCustomExtension(
  features: ComposerExtensionOptions["features"],
  slot: ComposerFeatureSlot,
): boolean {
  const override = features?.[slot];
  return override !== undefined && override !== false;
}

/**
 * Same WYSIWYG model as the dashboard SimpleEditor. Links use the kit's
 * colored underline with exclusive typing; files stay chips; prompt tokens
 * render as inline mentions.
 */
export function createComposerExtensions(
  options: ComposerExtensionOptions = {},
): AnyExtension[] {
  const { features, extraExtensions = [] } = options;
  const useStarterCodeBlock =
    features?.codeBlock !== false && !isCustomExtension(features, "codeBlock");

  const extensions: AnyExtension[] = [
    StarterKit.configure({
      heading: { levels: [...COMPOSER_HEADING_LEVELS] },
      codeBlock: useStarterCodeBlock ? {} : false,
      dropcursor: false,
      gapcursor: false,
      trailingNode: false,
      horizontalRule: false,
      link: false,
      strike: false,
      code: false,
      bold: false,
      italic: false,
      underline: false,
    }),
    HorizontalRule,
    ComposerBold,
    ComposerItalic,
    ComposerStrike,
    ComposerCode,
    ComposerUnderline,
  ];

  if (isCustomExtension(features, "codeBlock") && features?.codeBlock) {
    extensions.push(features.codeBlock);
  }

  const taskList = resolveSlot(features, "taskList", TaskList);
  if (taskList !== null) {
    extensions.push(taskList, ComposerTaskItem);
  }

  const highlight = resolveSlot(features, "highlight", ComposerHighlight);
  if (highlight !== null) {
    extensions.push(highlight);
  }

  const link = resolveSlot(features, "link", ComposerLink);
  if (link !== null) {
    extensions.push(link);
  }

  const fileChip = resolveSlot(features, "fileChip", ComposerFile);
  if (fileChip !== null) {
    extensions.push(fileChip);
  }

  const promptToken = resolveSlot(features, "promptToken", PromptToken);
  if (promptToken !== null) {
    extensions.push(promptToken);
  }

  extensions.push(
    ComposerChipSelection,
    ComposerNewline,
    ComposerCodeFence,
    ComposerMarkdownPaste,
    ComposerMarkdownBackfill,
    ComposerMarkdownRevert,
    ComposerMarkStartTyping,
    Placeholder.configure({
      placeholder: (props) => placeholderText(options, props),
      showOnlyCurrent: true,
      showOnlyWhenEditable: true,
      emptyEditorClass: "is-editor-empty",
      emptyNodeClass: "is-empty",
    }),
    ...extraExtensions,
  );

  return extensions;
}
