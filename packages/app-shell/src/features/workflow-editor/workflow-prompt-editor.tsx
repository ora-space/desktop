import { useEffect, useId, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import type { JSONContent } from "@tiptap/core";
import {
  IconArrowsMaximize,
  IconCopy,
  IconVariable,
} from "@tabler/icons-react";
import { markdownToComposerContent } from "@ora/editor/composer";
import {
  Button,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@ora/ui";
import {
  validReferences,
  type WorkflowVariableCatalogEntry,
} from "@ora/workflow-mock";
import {
  ComposerEditor,
  type ComposerEditorHandle,
} from "../editor/composer-editor";
import {
  workflowVariableLabel,
  workflowVariableTokenLabel,
} from "./workflow-variable-label";
import type { EditorWorkflowVariable } from "./workflow-variable-display";
import { groupWorkflowVariables } from "./workflow-variable-groups";
import { WorkflowVariableRowContent } from "./workflow-variable-list";
import { WorkflowPromptVariableToken } from "./workflow-prompt-token-extension";

interface WorkflowPromptEditorProps {
  value: string;
  variableCatalog: WorkflowVariableCatalogEntry[];
  ariaLabel: string;
  insertVariableLabel: string;
  onChange: (value: string) => void;
  /** Compact single-line field used by condition comparison values. */
  compact?: boolean;
  placeholder?: string;
}

interface VariableMenuPosition {
  left: number;
  top: number;
}

/** Edits an Agent prompt with caret-anchored variable insertion and compact text tools. */
export function WorkflowPromptEditor({
  value,
  variableCatalog,
  ariaLabel,
  insertVariableLabel,
  onChange,
  compact = false,
  placeholder,
}: WorkflowPromptEditorProps) {
  const { t } = useTranslation();
  const menuId = `workflow-variable-menu-${useId().replaceAll(":", "")}`;
  const globalVariablesLabel = t("settings.workflow.globalVariables");
  const editorRef = useRef<ComposerEditorHandle>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const preservedSelectionRef = useRef<{ from: number; to: number } | null>(
    null,
  );
  const [characterCount, setCharacterCount] = useState(value.length);
  const [expanded, setExpanded] = useState(false);
  const [slashQuery, setSlashQuery] = useState<string | null>(null);
  const [highlightedIndex, setHighlightedIndex] = useState(0);
  const [highlightQuery, setHighlightQuery] = useState<string | null>(null);
  const [menuPosition, setMenuPosition] = useState<VariableMenuPosition>({
    left: 8,
    top: 68,
  });
  const initialDocument = useMemo(
    () => labeledVariableDocument(value, variableCatalog, globalVariablesLabel),
    [globalVariablesLabel, value, variableCatalog],
  );
  const visibleVariables = useMemo(() => {
    const query = slashQuery?.trim().toLocaleLowerCase() ?? "";
    if (query === "") {
      return variableCatalog;
    }
    return variableCatalog.filter((variable) => {
      const selector = variable.selector.join(".").toLocaleLowerCase();
      return (
        selector.includes(query) ||
        workflowVariableLabel(variable).toLocaleLowerCase().includes(query)
      );
    });
  }, [slashQuery, variableCatalog]);
  const variableGroups = useMemo(
    () => groupWorkflowVariables(visibleVariables, globalVariablesLabel),
    [globalVariablesLabel, visibleVariables],
  );
  // Filter and open/close reset the highlight to the first visible row, matching Dify.
  if (slashQuery !== highlightQuery) {
    setHighlightQuery(slashQuery);
    setHighlightedIndex(0);
  }
  if (
    visibleVariables.length > 0 &&
    highlightedIndex >= visibleVariables.length
  ) {
    setHighlightedIndex(visibleVariables.length - 1);
  }
  const activeOptionId =
    slashQuery !== null && visibleVariables[highlightedIndex] !== undefined
      ? `${menuId}-option-${highlightedIndex}`
      : undefined;

  /** Replaces the active slash query with one serialized variable token. */
  const insertVariable = (variable: WorkflowVariableCatalogEntry): void => {
    const meta = variableTokenMeta(variable, globalVariablesLabel);
    editorRef.current?.insertPromptToken(
      "variable",
      variable.selector.join("."),
      workflowVariableTokenLabel(variable),
      meta,
    );
    setSlashQuery(null);
  };

  /**
   * Arrow/Enter stay on the picker while it is open so Enter confirms a variable
   * instead of inserting a newline the way the prompt field normally does.
   */
  const handleMenuKeyDown = (event: KeyboardEvent): boolean => {
    if (slashQuery === null) {
      return false;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (visibleVariables.length === 0) {
        return true;
      }
      const direction = event.key === "ArrowDown" ? 1 : -1;
      setHighlightedIndex(
        (current) =>
          (current + direction + visibleVariables.length) %
          visibleVariables.length,
      );
      return true;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      setSlashQuery(null);
      return true;
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      const variable = visibleVariables[highlightedIndex];
      if (variable !== undefined) {
        insertVariable(variable);
      }
      return true;
    }
    return false;
  };

  useEffect(() => {
    if (slashQuery === null || activeOptionId === undefined) {
      return;
    }
    document
      .getElementById(activeOptionId)
      ?.scrollIntoView({ block: "nearest" });
  }, [activeOptionId, slashQuery]);

  /**
   * Places the picker in the viewport below the caret so overflow-hidden
   * condition cards cannot clip it or grow around it.
   */
  const positionVariableMenu = (): void => {
    const panel = panelRef.current?.getBoundingClientRect();
    const caret = editorRef.current?.getCaretRect();
    const anchor = caret ?? panel;
    if (anchor === undefined || anchor === null) {
      setMenuPosition({ left: 8, top: 8 });
      return;
    }
    const menuWidth = 256;
    const menuHeight = 224;
    const gap = 4;
    const left = Math.max(
      8,
      Math.min(anchor.left, window.innerWidth - menuWidth - 8),
    );
    const below = anchor.bottom + gap;
    const top =
      below + menuHeight > window.innerHeight - 8
        ? Math.max(8, anchor.top - menuHeight - gap)
        : below;
    setMenuPosition({ left, top });
  };

  useEffect(() => {
    if (slashQuery === null) {
      return;
    }
    const frame = requestAnimationFrame(() => positionVariableMenu());
    const update = (): void => positionVariableMenu();
    window.addEventListener("resize", update);
    document.addEventListener("scroll", update, true);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", update);
      document.removeEventListener("scroll", update, true);
    };
  }, [slashQuery]);

  const panel = (
    <div
      ref={panelRef}
      role="group"
      aria-invalid={!validReferences(value, variableCatalog)}
      className={
        compact
          ? "relative"
          : expanded
            ? "relative flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-input bg-background"
            : "relative rounded-lg border border-input bg-background"
      }
    >
      {!compact && (
        <div className="flex h-9 shrink-0 items-center justify-end gap-0.5 border-b border-border bg-muted/35 px-1.5">
          <span
            className="mr-auto px-1.5 text-[11px] tabular-nums text-muted-foreground"
            aria-label={t("settings.workflow.field.promptCharacterCount", {
              count: characterCount,
            })}
          >
            {characterCount}
          </span>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="size-7"
            disabled={variableCatalog.length === 0}
            aria-label={insertVariableLabel}
            title={insertVariableLabel}
            onMouseDown={(event) => {
              preservedSelectionRef.current =
                editorRef.current?.getSelection() ?? null;
              // Keeping browser focus in the editor preserves the visible caret while the button
              // still receives its subsequent click event.
              event.preventDefault();
            }}
            onClick={() => {
              editorRef.current?.insertText(
                "/",
                preservedSelectionRef.current ?? undefined,
              );
              preservedSelectionRef.current = null;
            }}
          >
            <IconVariable className="size-4" />
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            className="size-7"
            aria-label={t("settings.workflow.field.copyPrompt")}
            title={t("settings.workflow.field.copyPrompt")}
            onClick={() => {
              void navigator.clipboard?.writeText(
                editorRef.current?.getText() ?? value,
              );
            }}
          >
            <IconCopy className="size-4" />
          </Button>
          {!expanded && (
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="size-7"
              aria-label={t("settings.workflow.field.expandPrompt")}
              title={t("settings.workflow.field.expandPrompt")}
              onClick={() => setExpanded(true)}
            >
              <IconArrowsMaximize className="size-4" />
            </Button>
          )}
        </div>
      )}
      {compact && (
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          className="absolute top-0.5 right-0.5 z-10 size-6"
          disabled={variableCatalog.length === 0}
          aria-label={insertVariableLabel}
          title={insertVariableLabel}
          onMouseDown={(event) => {
            preservedSelectionRef.current =
              editorRef.current?.getSelection() ?? null;
            event.preventDefault();
          }}
          onClick={() => {
            editorRef.current?.insertText(
              "/",
              preservedSelectionRef.current ?? undefined,
            );
            preservedSelectionRef.current = null;
          }}
        >
          <IconVariable className="size-3.5" />
        </Button>
      )}
      <ComposerEditor
        ref={editorRef}
        id={compact ? undefined : "workflow-agent-prompt"}
        initialDocument={initialDocument}
        promptTokenExtension={WorkflowPromptVariableToken}
        placeholder={placeholder}
        enterKey={compact ? "submit" : "newline"}
        className={
          compact
            ? "workflow-compact-value h-8 min-h-8 overflow-hidden border-0 bg-transparent pr-7 text-xs leading-5 [&_.composer-editor-content]:!px-1.5 [&_.composer-editor-content]:!py-0"
            : expanded
              ? // Shell scrolls; tip tap must not keep overflow:auto or it swallows the wheel.
                "min-h-0 flex-1 overflow-y-auto border-0 bg-transparent text-xs leading-5 [&_.tiptap]:min-h-full [&_.tiptap]:!max-h-none [&_.tiptap]:!overflow-y-visible"
              : "min-h-32 border-0 bg-transparent text-xs leading-5 [&_.tiptap]:min-h-32 [&_.tiptap]:!max-h-64"
        }
        ariaLabel={ariaLabel}
        ariaAutoComplete="list"
        ariaHasPopup="listbox"
        ariaExpanded={slashQuery !== null}
        ariaControls={slashQuery !== null ? menuId : undefined}
        ariaActivedescendant={activeOptionId}
        slashQueryMode="inline"
        onMenuKeyDown={handleMenuKeyDown}
        onSubmit={() => undefined}
        onTextChange={(text) => {
          setCharacterCount(text.length);
          onChange(text);
        }}
        onQueryChange={(query) => {
          setSlashQuery(query.slashQuery);
        }}
      />
      {slashQuery !== null &&
        createPortal(
          <div
            id={menuId}
            role="listbox"
            aria-label={insertVariableLabel}
            className="fixed z-[200] max-h-56 w-64 overflow-y-auto rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-lg"
            style={{ left: menuPosition.left, top: menuPosition.top }}
          >
            {variableGroups.map((group) => (
              <section key={group.label} aria-label={group.label}>
                <p className="px-2 pt-1.5 pb-0.5 text-[11px] font-medium text-muted-foreground">
                  {group.label}
                </p>
                {group.variables.map((variable) => {
                  const optionIndex = visibleVariables.findIndex(
                    (candidate) =>
                      candidate.selector.join(".") ===
                      variable.selector.join("."),
                  );
                  const selected = optionIndex === highlightedIndex;
                  return (
                    <button
                      key={variable.selector.join(".")}
                      id={`${menuId}-option-${optionIndex}`}
                      type="button"
                      role="option"
                      aria-label={workflowVariableLabel(variable)}
                      aria-selected={selected}
                      className="flex w-full items-center justify-between gap-3 rounded-md px-2 py-1.5 text-left text-xs shadow-none aria-selected:bg-background aria-selected:shadow-sm"
                      onMouseDown={(event) => event.preventDefault()}
                      onMouseEnter={() => setHighlightedIndex(optionIndex)}
                      onClick={() => insertVariable(variable)}
                    >
                      <WorkflowVariableRowContent variable={variable} />
                    </button>
                  );
                })}
              </section>
            ))}
            {visibleVariables.length === 0 && (
              <p className="px-2 py-3 text-center text-xs text-muted-foreground">
                {insertVariableLabel}
              </p>
            )}
          </div>,
          document.body,
        )}
    </div>
  );

  if (compact) {
    return panel;
  }

  return (
    <>
      {!expanded && panel}
      <Dialog open={expanded} onOpenChange={setExpanded}>
        <DialogContent className="flex max-h-[85vh] w-full max-w-5xl flex-col overflow-hidden sm:max-w-5xl">
          <DialogHeader className="shrink-0">
            <DialogTitle>{ariaLabel}</DialogTitle>
          </DialogHeader>
          {expanded && panel}
        </DialogContent>
      </Dialog>
    </>
  );
}

/** Adds current node titles to persisted variable tokens without changing their selectors. */
function labeledVariableDocument(
  value: string,
  catalog: WorkflowVariableCatalogEntry[],
  globalVariablesLabel: string,
): JSONContent {
  const variablesBySelector = new Map(
    catalog.map((variable) => [
      variable.selector.join("."),
      {
        label: workflowVariableTokenLabel(variable),
        meta: variableTokenMeta(variable, globalVariablesLabel),
      },
    ]),
  );
  const enrich = (node: JSONContent): JSONContent => {
    const content = node.content?.map(enrich);
    if (node.type !== "promptToken" || node.attrs?.kind !== "variable") {
      return content === undefined ? node : { ...node, content };
    }
    const name = String(node.attrs.name ?? "");
    const variable = variablesBySelector.get(name);
    return {
      ...node,
      attrs: {
        ...node.attrs,
        label: variable?.label ?? name,
        meta: variable?.meta ?? "",
      },
      ...(content === undefined ? {} : { content }),
    };
  };
  return enrich(markdownToComposerContent(value));
}

/** Serializes the presentation-only data needed to render a workflow variable token. */
function variableTokenMeta(
  variable: WorkflowVariableCatalogEntry,
  globalVariablesLabel: string,
): string {
  return JSON.stringify({
    kind: (variable as EditorWorkflowVariable).sourceNodeKind,
    node:
      variable.sourceNodeTitle ??
      (variable.scope === "global"
        ? globalVariablesLabel
        : variable.sourceNodeId),
    variableName: variable.variableName,
  });
}
