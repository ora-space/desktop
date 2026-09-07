// --- Utils ---
export {
  cn,
  isMac,
  formatShortcutKey,
  parseShortcutKeys,
  isMarkInSchema,
  isNodeInSchema,
  isValidPosition,
  isExtensionAvailable,
  findNodeAtPosition,
  findNodePosition,
  isNodeTypeSelected,
  isAllowedUri,
  sanitizeUrl,
} from "./utils.ts";

// --- Diff ---
export { diffJSONContent } from "./diff.ts";
export type { DiffStatus } from "./diff.ts";

// --- Hooks ---
export { useTiptapEditor } from "./hooks/use-tiptap-editor.ts";
export { useIsMobile } from "./hooks/use-mobile.ts";
export { useMenuNavigation } from "./hooks/use-menu-navigation.ts";

// --- Icons ---
export { AlignCenterIcon } from "./icons/align-center-icon.tsx";
export { AlignJustifyIcon } from "./icons/align-justify-icon.tsx";
export { AlignLeftIcon } from "./icons/align-left-icon.tsx";
export { AlignRightIcon } from "./icons/align-right-icon.tsx";
export { ArrowLeftIcon } from "./icons/arrow-left-icon.tsx";
export { BanIcon } from "./icons/ban-icon.tsx";
export { BlockquoteIcon } from "./icons/blockquote-icon.tsx";
export { BoldIcon } from "./icons/bold-icon.tsx";
export { ChevronDownIcon } from "./icons/chevron-down-icon.tsx";
export { CloseIcon } from "./icons/close-icon.tsx";
export { CodeBlockIcon } from "./icons/code-block-icon.tsx";
export { Code2Icon } from "./icons/code2-icon.tsx";
export { CornerDownLeftIcon } from "./icons/corner-down-left-icon.tsx";
export { ExternalLinkIcon } from "./icons/external-link-icon.tsx";
export { HeadingFiveIcon } from "./icons/heading-five-icon.tsx";
export { HeadingFourIcon } from "./icons/heading-four-icon.tsx";
export { HeadingIcon } from "./icons/heading-icon.tsx";
export { HeadingOneIcon } from "./icons/heading-one-icon.tsx";
export { HeadingSixIcon } from "./icons/heading-six-icon.tsx";
export { HeadingThreeIcon } from "./icons/heading-three-icon.tsx";
export { HeadingTwoIcon } from "./icons/heading-two-icon.tsx";
export { HighlighterIcon } from "./icons/highlighter-icon.tsx";
export { ImagePlusIcon } from "./icons/image-plus-icon.tsx";
export { ItalicIcon } from "./icons/italic-icon.tsx";
export { LinkIcon } from "./icons/link-icon.tsx";
export { ListIcon } from "./icons/list-icon.tsx";
export { ListOrderedIcon } from "./icons/list-ordered-icon.tsx";
export { ListTodoIcon } from "./icons/list-todo-icon.tsx";
export { MoonStarIcon } from "./icons/moon-star-icon.tsx";
export { Redo2Icon } from "./icons/redo2-icon.tsx";
export { RestoreIcon } from "./icons/restore-icon.tsx";
export { StrikeIcon } from "./icons/strike-icon.tsx";
export { SubscriptIcon } from "./icons/subscript-icon.tsx";
export { SunIcon } from "./icons/sun-icon.tsx";
export { SuperscriptIcon } from "./icons/superscript-icon.tsx";
export { TrashIcon } from "./icons/trash-icon.tsx";
export { UnderlineIcon } from "./icons/underline-icon.tsx";
export { Undo2Icon } from "./icons/undo2-icon.tsx";

// --- Primitives ---
export * from "./primitive/badge/index.tsx";
export * from "./primitive/button/index.tsx";
export * from "./primitive/card/index.tsx";
export * from "./primitive/dropdown-menu/index.tsx";
export * from "./primitive/input/index.tsx";
export * from "./primitive/popover/index.tsx";
export * from "./primitive/separator/index.tsx";
export * from "./primitive/spacer/index.tsx";
export * from "./primitive/toolbar/index.tsx";
export * from "./primitive/tooltip/index.tsx";

// --- Nodes ---
export * from "./node/diff-node/index.ts";
export * from "./node/image-upload-node/index.tsx";
export * from "./node/toc-node/index.ts";
export * from "./node/video-node/index.ts";
export { CachedImage } from "./node/image-node/cached-image-extension.ts";
export type { CachedImageOptions } from "./node/image-node/cached-image-extension.ts";
export { HorizontalRule } from "./node/horizontal-rule-node/horizontal-rule-node-extension.ts";

// --- UI Components (explicit to avoid name conflicts across modules) ---
export { BlockquoteButton } from "./ui/blockquote-button/blockquote-button.tsx";
export { useBlockquote } from "./ui/blockquote-button/use-blockquote.ts";

export { CodeBlockButton } from "./ui/code-block-button/code-block-button.tsx";
export { useCodeBlock } from "./ui/code-block-button/use-code-block.ts";

export { ColorHighlightButton } from "./ui/color-highlight-button/color-highlight-button.tsx";
export { useColorHighlight } from "./ui/color-highlight-button/use-color-highlight.ts";

export {
  ColorHighlightPopover,
  ColorHighlightPopoverContent,
  ColorHighlightPopoverButton,
} from "./ui/color-highlight-popover/color-highlight-popover.tsx";

export { HeadingButton } from "./ui/heading-button/heading-button.tsx";
export { useHeading } from "./ui/heading-button/use-heading.ts";

export { HeadingDropdownMenu } from "./ui/heading-dropdown-menu/heading-dropdown-menu.tsx";
export { useHeadingDropdownMenu } from "./ui/heading-dropdown-menu/use-heading-dropdown-menu.ts";

export {
  LinkPopover,
  LinkContent,
  LinkButton,
} from "./ui/link-popover/link-popover.tsx";
export { useLinkPopover } from "./ui/link-popover/use-link-popover.ts";

export { ListButton } from "./ui/list-button/list-button.tsx";
export { useList } from "./ui/list-button/use-list.ts";

export { ListDropdownMenu } from "./ui/list-dropdown-menu/list-dropdown-menu.tsx";
export { useListDropdownMenu } from "./ui/list-dropdown-menu/use-list-dropdown-menu.ts";

export { MarkButton } from "./ui/mark-button/mark-button.tsx";
export { useMark } from "./ui/mark-button/use-mark.ts";

export { TextAlignButton } from "./ui/text-align-button/text-align-button.tsx";
export { useTextAlign } from "./ui/text-align-button/use-text-align.ts";

export { UndoRedoButton } from "./ui/undo-redo-button/undo-redo-button.tsx";
export { useUndoRedo } from "./ui/undo-redo-button/use-undo-redo.ts";

// --- Editor ---
export { ThemeToggle } from "./editor/theme-toggle.tsx";

// --- Composer preset ---
export {
  COMPOSER_CAPABILITIES,
  createComposerExtensions,
} from "./composer/create-composer-extensions.ts";
export type {
  ComposerExtensionOptions,
  ComposerFeatureSlot,
  ComposerPlaceholderProps,
} from "./composer/create-composer-extensions.ts";
export {
  documentPlainText,
  plainTextToComposerContent,
} from "./composer/composer-plain-text.ts";
export { PromptToken } from "./composer/prompt-token.ts";
export type { PromptTokenKind } from "./composer/prompt-token.ts";
export { ComposerNewline } from "./composer/composer-newline.ts";
export { ComposerLink } from "./composer/composer-link.ts";
export { ComposerFile } from "./composer/composer-file.ts";
export type { ComposerFileAttrs } from "./composer/composer-file.ts";
