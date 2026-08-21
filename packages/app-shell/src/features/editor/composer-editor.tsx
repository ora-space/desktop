import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
} from "react";
import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import type { EditorView } from "@tiptap/pm/view";
import {
  createComposerExtensions,
  documentPlainText,
  markdownToComposerContent,
  resolveComposerEnter,
  resolveComposerLinkHref,
  type ComposerFileAttrs,
  type PromptTokenKind,
} from "@ora/editor/composer";
import type { JSONContent } from "@tiptap/core";
import { useOptionalPlatform } from "@ora/app-shell/platform";
import { cn } from "@ora/ui";
import {
  AT_TRIGGER_PATTERN,
  EMPTY_COMPOSER_QUERY,
  SLASH_TRIGGER_PATTERN,
  queryStateFromText,
  queryStatesEqual,
  type ComposerQueryState,
} from "./composer-query";
import { AppComposerFile } from "./composer-file-extension";
import "./composer-editor.css";

export interface ComposerEditorHandle {
  getText: () => string;
  /** TipTap JSON so chip attrs (path, kind, token name) survive park/restore. */
  getJSON: () => JSONContent;
  focus: (options?: {
    preventScroll?: boolean;
    /** Defaults to end; `keep` restores focus without moving the caret. */
    at?: "start" | "end" | "keep";
  }) => void;
  clear: () => void;
  replaceText: (text: string) => void;
  /** Restores a parked TipTap document without round-tripping through plain text. */
  replaceDocument: (doc: JSONContent) => void;
  insertPromptToken: (kind: PromptTokenKind, name: string) => void;
  insertFileChips: (files: ComposerFileAttrs[]) => void;
  appendText: (text: string) => void;
  removeAtToken: () => void;
  getDom: () => HTMLElement | null;
}

export interface ComposerEditorProps {
  placeholder?: string;
  disabled?: boolean;
  autoFocus?: boolean;
  /** Seed from `documentPlainText` so HITL drafts restore as the same nodes. */
  initialText?: string;
  enterKey?: "submit" | "newline";
  className?: string;
  /** Optional DOM id so a `<label htmlFor>` can focus the contenteditable. */
  id?: string;
  ariaLabel?: string;
  ariaAutoComplete?: "list" | "none";
  ariaHasPopup?: "listbox";
  ariaExpanded?: boolean;
  ariaControls?: string;
  ariaActivedescendant?: string;
  onSubmit: () => void;
  /**
   * Fires only when slash/@/blankness actually change so typing a normal
   * sentence does not re-render the surrounding composer chrome.
   */
  onQueryChange?: (query: ComposerQueryState) => void;
  /** Document edits only. Dismisses the plus menu without storing draft text. */
  onDocChange?: () => void;
  /** Draft text for HITL persistence. Chat omits this so typing does not re-render chrome. */
  onTextChange?: (text: string) => void;
  onPasteFiles?: (files: File[]) => void;
  /** Return true when the action menu consumed the key. */
  onMenuKeyDown?: (event: KeyboardEvent) => boolean;
}

/**
 * Uncontrolled Tiptap prompt field. The editor is the source of truth; React
 * is notified only for menu queries, optional draft sync, and submit.
 */
export const ComposerEditor = forwardRef<
  ComposerEditorHandle,
  ComposerEditorProps
>(function ComposerEditor(
  {
    placeholder = "",
    disabled = false,
    autoFocus = false,
    initialText = "",
    enterKey = "submit",
    className,
    id,
    ariaLabel,
    ariaAutoComplete,
    ariaHasPopup,
    ariaExpanded,
    ariaControls,
    ariaActivedescendant,
    onSubmit,
    onQueryChange,
    onDocChange,
    onTextChange,
    onPasteFiles,
    onMenuKeyDown,
  },
  ref,
) {
  const placeholderRef = useRef(placeholder);
  placeholderRef.current = placeholder;
  const onSubmitRef = useRef(onSubmit);
  onSubmitRef.current = onSubmit;
  const onQueryChangeRef = useRef(onQueryChange);
  onQueryChangeRef.current = onQueryChange;
  const onDocChangeRef = useRef(onDocChange);
  onDocChangeRef.current = onDocChange;
  const onTextChangeRef = useRef(onTextChange);
  onTextChangeRef.current = onTextChange;
  const onPasteFilesRef = useRef(onPasteFiles);
  onPasteFilesRef.current = onPasteFiles;
  const onMenuKeyDownRef = useRef(onMenuKeyDown);
  onMenuKeyDownRef.current = onMenuKeyDown;
  const enterKeyRef = useRef(enterKey);
  enterKeyRef.current = enterKey;
  const lastQueryRef = useRef<ComposerQueryState>(EMPTY_COMPOSER_QUERY);
  const suppressNotifyRef = useRef(false);
  const platform = useOptionalPlatform();
  const platformRef = useRef(platform);
  platformRef.current = platform;

  const extensions = useMemo(
    () =>
      createComposerExtensions({
        placeholder: () => placeholderRef.current,
        features: { fileChip: AppComposerFile },
      }),
    [],
  );

  const editor = useEditor({
    extensions,
    content: markdownToComposerContent(initialText),
    autofocus: autoFocus ? "end" : false,
    editable: !disabled,
    immediatelyRender: true,
    shouldRerenderOnTransaction: false,
    editorProps: {
      attributes: {
        class: "composer-editor-content px-2 py-1",
        role: "textbox",
        "aria-multiline": "true",
      },
      // Keep the caret inside the 200px clip without letting jsdom's missing
      // layout boxes fail tests (coordsAtPos throws without getClientRects).
      handleScrollToSelection: scrollComposerSelectionIntoView,
      handleKeyDown: (view, event) => {
        if (event.isComposing || event.keyCode === 229) {
          return false;
        }
        if (onMenuKeyDownRef.current?.(event) === true) {
          return true;
        }
        if (
          event.key === "Enter" &&
          !event.shiftKey &&
          !event.altKey &&
          !event.ctrlKey &&
          !event.metaKey
        ) {
          const action = resolveComposerEnter(view);
          if (action === "handled") {
            event.preventDefault();
            return true;
          }
          if (enterKeyRef.current === "submit") {
            event.preventDefault();
            onSubmitRef.current();
            return true;
          }
        }
        return false;
      },
    },
    onCreate: ({ editor: current }) => {
      emitQuery(current, lastQueryRef, onQueryChangeRef);
    },
    onUpdate: ({ editor: current }) => {
      if (suppressNotifyRef.current) {
        syncComposerTextDataset(current, lastQueryRef);
        return;
      }
      emitQuery(current, lastQueryRef, onQueryChangeRef);
      onDocChangeRef.current?.();
      onTextChangeRef.current?.(documentPlainText(current.state.doc));
    },
    onSelectionUpdate: ({ editor: current }) => {
      if (suppressNotifyRef.current) {
        syncComposerTextDataset(current, lastQueryRef);
        return;
      }
      emitQuery(current, lastQueryRef, onQueryChangeRef);
    },
  });

  useEffect(() => {
    editor.setEditable(!disabled);
  }, [disabled, editor]);

  useEffect(() => {
    const dom = editor.view.dom;
    setOptionalAttr(dom, "id", id);
    setOptionalAttr(dom, "aria-label", ariaLabel);
    setOptionalAttr(dom, "aria-autocomplete", ariaAutoComplete);
    setOptionalAttr(dom, "aria-haspopup", ariaHasPopup);
    setOptionalAttr(dom, "aria-controls", ariaControls);
    setOptionalAttr(dom, "aria-activedescendant", ariaActivedescendant);
    if (ariaExpanded === undefined) {
      dom.removeAttribute("aria-expanded");
    } else {
      dom.setAttribute("aria-expanded", ariaExpanded ? "true" : "false");
    }
    if (disabled) {
      dom.setAttribute("aria-disabled", "true");
    } else {
      dom.removeAttribute("aria-disabled");
    }
  }, [
    ariaActivedescendant,
    ariaAutoComplete,
    ariaControls,
    ariaExpanded,
    ariaHasPopup,
    ariaLabel,
    disabled,
    editor,
    id,
  ]);

  useImperativeHandle(
    ref,
    () => ({
      getText: () => documentPlainText(editor.state.doc),
      focus: (options) => {
        const at = options?.at ?? "end";
        if (at === "keep") {
          if (options?.preventScroll === true) {
            editor.view.dom.focus({ preventScroll: true });
            return;
          }
          editor.commands.focus();
          return;
        }
        if (options?.preventScroll === true) {
          editor.view.dom.focus({ preventScroll: true });
          if (at === "start") {
            editor.commands.focus("start");
          }
          return;
        }
        editor.commands.focus(at);
      },
      getJSON: () => editor.getJSON(),
      clear: () => {
        withSuppressedNotify(suppressNotifyRef, () => {
          editor.commands.clearContent(true);
        });
      },
      replaceText: (text) => {
        withSuppressedNotify(suppressNotifyRef, () => {
          editor
            .chain()
            .setContent(markdownToComposerContent(text))
            .focus("end")
            .run();
        });
      },
      replaceDocument: (doc) => {
        withSuppressedNotify(suppressNotifyRef, () => {
          editor.chain().setContent(doc).focus("end").run();
        });
      },
      insertPromptToken: (kind, name) => {
        deleteTriggerToken(editor, SLASH_TRIGGER_PATTERN);
        editor.commands.setPromptToken(kind, name);
      },
      insertFileChips: (files) => {
        // Leave the caret where insertContent placed it so mid-prompt @ mentions
        // do not jump to the document end. Callers that want the end (sidebar
        // inject) should follow with focus({ at: "end" }).
        editor.commands.insertComposerFiles(files);
      },
      appendText: (text) => {
        // Insert parsed blocks at the end so existing `/command` chips stay
        // nodes. A full documentPlainText round-trip cannot rebuild slash chips.
        const blocks = markdownToComposerContent(text).content ?? [];
        if (blocks.length === 0) {
          return;
        }
        const content = editor.isEmpty
          ? blocks
          : [{ type: "paragraph" }, ...blocks];
        editor.chain().focus("end").insertContent(content).run();
      },
      removeAtToken: () => {
        deleteTriggerToken(editor, AT_TRIGGER_PATTERN);
      },
      getDom: () => editor.view.dom,
    }),
    [editor],
  );

  return (
    <div
      data-slot="composer-editor"
      className={cn(
        "composer-editor-shell",
        disabled && "pointer-events-none opacity-50",
        className,
      )}
      onPointerDownCapture={(event) => {
        const adapter = platformRef.current;
        if (adapter === null) {
          return;
        }
        const safe = resolveComposerLinkHref(event);
        if (safe === null) {
          return;
        }
        // Consume the press before ProseMirror places a caret or Tiptap waits
        // for mouseup; Desktop `openExternalUrl` is fire-and-forget from here.
        event.preventDefault();
        event.stopPropagation();
        void adapter.openExternalUrl(safe);
      }}
      onPasteCapture={(event) => {
        const files = [...(event.clipboardData?.files ?? [])];
        if (files.length === 0 || onPasteFilesRef.current === undefined) {
          return;
        }
        const text =
          typeof event.clipboardData?.getData === "function"
            ? event.clipboardData.getData("text/plain")
            : "";
        // Always own the paste when files are present so the image path runs;
        // still insert accompanying plain text so markdown+screenshot copies
        // do not lose the typed payload.
        event.preventDefault();
        event.stopPropagation();
        onPasteFilesRef.current(files);
        if (text.length > 0) {
          editor
            .chain()
            .focus()
            .insertContent(markdownToComposerContent(text).content ?? [])
            .run();
        }
      }}
    >
      <EditorContent editor={editor} />
    </div>
  );
});

ComposerEditor.displayName = "ComposerEditor";

function deleteTriggerToken(editor: Editor, pattern: RegExp): void {
  const { $from } = editor.state.selection;
  const textBefore = $from.parent.textBetween(
    0,
    $from.parentOffset,
    undefined,
    "\n",
  );
  const match = textBefore.match(pattern);
  if (match?.index === undefined) {
    return;
  }
  editor
    .chain()
    .focus()
    .deleteRange({ from: $from.start() + match.index, to: $from.pos })
    .run();
}

/** Writes live query state to the DOM for tests without a React update. */
function syncComposerTextDataset(
  editor: Editor,
  lastQueryRef: { current: ComposerQueryState },
): void {
  const text = documentPlainText(editor.state.doc);
  const before = editor.state.doc.textBetween(
    0,
    editor.state.selection.from,
    "\n",
    "\n",
  );
  editor.view.dom.dataset.composerText = text;
  lastQueryRef.current = queryStateFromText(text, before);
}

/**
 * Programmatic setContent uses flushSync via React node views. Callers that
 * must run outside React's commit (session hydrate) wrap with this so those
 * transactions do not setState the parent composer.
 */
function withSuppressedNotify(
  suppressNotifyRef: { current: boolean },
  run: () => void,
): void {
  suppressNotifyRef.current = true;
  try {
    run();
  } finally {
    suppressNotifyRef.current = false;
  }
}

/** Writes live query state to the DOM for tests without a React update. */
function emitQuery(
  editor: Editor,
  lastQueryRef: { current: ComposerQueryState },
  onQueryChangeRef: { current: ComposerEditorProps["onQueryChange"] },
): void {
  const text = documentPlainText(editor.state.doc);
  const before = editor.state.doc.textBetween(
    0,
    editor.state.selection.from,
    "\n",
    "\n",
  );
  editor.view.dom.dataset.composerText = text;
  const next = queryStateFromText(text, before);
  if (editor.state.selection.$from.parent.type.name === "codeBlock") {
    next.slashQuery = null;
    next.atQuery = null;
    next.atTriggerIndex = null;
  }
  if (queryStatesEqual(lastQueryRef.current, next)) {
    return;
  }
  lastQueryRef.current = next;
  onQueryChangeRef.current?.(next);
}

function setOptionalAttr(
  element: HTMLElement,
  name: string,
  value: string | undefined,
): void {
  if (value === undefined) {
    element.removeAttribute(name);
    return;
  }
  element.setAttribute(name, value);
}

/**
 * Scrolls the composer clip so the caret stays visible. Returns true so
 * ProseMirror does not fall back to its default coordsAtPos path.
 */
function scrollComposerSelectionIntoView(view: EditorView): boolean {
  try {
    const coords = view.coordsAtPos(view.state.selection.head);
    const scroller = view.dom;
    const rect = scroller.getBoundingClientRect();
    const pad = 8;
    if (coords.bottom > rect.bottom - pad) {
      scroller.scrollTop += coords.bottom - rect.bottom + pad;
    } else if (coords.top < rect.top + pad) {
      scroller.scrollTop -= rect.top - coords.top + pad;
    }
  } catch {
    // jsdom has no layout boxes for coordsAtPos.
  }
  return true;
}
