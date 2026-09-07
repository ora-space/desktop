import { useMemo, useRef, useState } from "react";
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
import type { WorkflowVariableCatalogEntry } from "@ora/workflow-mock";
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
}: WorkflowPromptEditorProps) {
  const { t } = useTranslation();
  const globalVariablesLabel = t("settings.workflow.globalVariables");
  const editorRef = useRef<ComposerEditorHandle>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const preservedSelectionRef = useRef<{ from: number; to: number } | null>(
    null,
  );
  const [characterCount, setCharacterCount] = useState(value.length);
  const [expanded, setExpanded] = useState(false);
  const [slashQuery, setSlashQuery] = useState<string | null>(null);
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

  /** Places the variable menu immediately below the editor's active slash caret. */
  const positionVariableMenu = (): void => {
    const panel = panelRef.current?.getBoundingClientRect();
    const caret = editorRef.current?.getCaretRect();
    if (
      panel === undefined ||
      panel.width <= 0 ||
      caret === null ||
      caret === undefined
    ) {
      setMenuPosition({ left: 8, top: 68 });
      return;
    }
    setMenuPosition({
      left: Math.max(8, Math.min(caret.left - panel.left, panel.width - 272)),
      top: caret.bottom - panel.top + 4,
    });
  };

  const panel = (
    <div
      ref={panelRef}
      className={
        expanded
          ? "relative flex min-h-[65vh] flex-col rounded-lg border border-input bg-background"
          : "relative rounded-lg border border-input bg-background"
      }
    >
      <div className="flex h-9 items-center justify-end gap-0.5 border-b border-border bg-muted/35 px-1.5">
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
      </div>
      <ComposerEditor
        ref={editorRef}
        id="workflow-agent-prompt"
        initialDocument={initialDocument}
        promptTokenExtension={WorkflowPromptVariableToken}
        enterKey="newline"
        className={
          expanded
            ? "min-h-0 flex-1 border-0 bg-transparent text-xs leading-5 [&_.tiptap]:h-full [&_.tiptap]:min-h-[55vh]"
            : "min-h-32 border-0 bg-transparent text-xs leading-5 [&_.tiptap]:min-h-32 [&_.tiptap]:max-h-64"
        }
        ariaLabel={ariaLabel}
        ariaAutoComplete="list"
        ariaHasPopup="listbox"
        ariaExpanded={slashQuery !== null}
        slashQueryMode="inline"
        onSubmit={() => undefined}
        onTextChange={(text) => {
          setCharacterCount(text.length);
          onChange(text);
        }}
        onQueryChange={(query) => {
          setSlashQuery(query.slashQuery);
          if (query.slashQuery !== null) {
            requestAnimationFrame(positionVariableMenu);
          }
        }}
      />
      {slashQuery !== null && (
        <div
          role="listbox"
          aria-label={insertVariableLabel}
          className="absolute z-50 max-h-56 w-64 overflow-y-auto rounded-lg border border-border bg-popover p-1 text-popover-foreground shadow-lg"
          style={{ left: menuPosition.left, top: menuPosition.top }}
        >
          {variableGroups.map((group) => (
            <section key={group.label} aria-label={group.label}>
              <p className="px-2 pt-1.5 pb-0.5 text-[11px] font-medium text-muted-foreground">
                {group.label}
              </p>
              {group.variables.map((variable) => (
                <button
                  key={variable.selector.join(".")}
                  type="button"
                  role="option"
                  aria-label={workflowVariableLabel(variable)}
                  aria-selected="false"
                  className="flex w-full items-center justify-between gap-3 rounded-md px-2 py-1.5 text-left text-xs hover:bg-accent hover:text-accent-foreground"
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => insertVariable(variable)}
                >
                  <WorkflowVariableRowContent variable={variable} />
                </button>
              ))}
            </section>
          ))}
          {visibleVariables.length === 0 && (
            <p className="px-2 py-3 text-center text-xs text-muted-foreground">
              {insertVariableLabel}
            </p>
          )}
        </div>
      )}
    </div>
  );

  return (
    <>
      {!expanded && panel}
      <Dialog open={expanded} onOpenChange={setExpanded}>
        <DialogContent className="max-w-5xl">
          <DialogHeader>
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
