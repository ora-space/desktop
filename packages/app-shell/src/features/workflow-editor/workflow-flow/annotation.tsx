import {
  createContext,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import { NodeResizer, type NodeProps } from "@xyflow/react";
import { IconTrash } from "@tabler/icons-react";
import {
  WORKFLOW_ANNOTATION_THEMES,
  type WorkflowAnnotationData,
  type WorkflowAnnotationNode,
  type WorkflowAnnotationTheme,
} from "@ora/workflow-mock";

interface WorkflowAnnotationActions {
  readOnly: boolean;
  update: (id: string, data: Partial<WorkflowAnnotationData>) => void;
  remove: (id: string) => void;
}

const WorkflowAnnotationActionsContext =
  createContext<WorkflowAnnotationActions | null>(null);

const THEME_CLASSES: Record<WorkflowAnnotationTheme, string> = {
  yellow:
    "border-amber-300/70 bg-amber-100 text-amber-950 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100",
  blue: "border-sky-300/70 bg-sky-100 text-sky-950 dark:border-sky-700 dark:bg-sky-950 dark:text-sky-100",
  green:
    "border-emerald-300/70 bg-emerald-100 text-emerald-950 dark:border-emerald-700 dark:bg-emerald-950 dark:text-emerald-100",
  pink: "border-rose-300/70 bg-rose-100 text-rose-950 dark:border-rose-700 dark:bg-rose-950 dark:text-rose-100",
  gray: "border-slate-300/70 bg-slate-100 text-slate-950 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-100",
};

interface WorkflowAnnotationActionsProviderProps {
  value: WorkflowAnnotationActions;
  children: ReactNode;
}

/** Supplies serializable annotation nodes with editor actions outside node.data. */
export function WorkflowAnnotationActionsProvider({
  value,
  children,
}: WorkflowAnnotationActionsProviderProps) {
  return (
    <WorkflowAnnotationActionsContext.Provider value={value}>
      {children}
    </WorkflowAnnotationActionsContext.Provider>
  );
}

/** Renders a resizable, themeable note that never exposes workflow handles. */
export function WorkflowAnnotationView({
  id,
  data,
  selected,
}: NodeProps<WorkflowAnnotationNode>) {
  const { t } = useTranslation();
  const actions = useContext(WorkflowAnnotationActionsContext);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const [editing, setEditing] = useState(false);
  if (actions === null) {
    throw new Error("WorkflowAnnotationView requires annotation actions");
  }
  const { readOnly } = actions;

  useEffect(() => {
    if (editing && selected && !readOnly) {
      const textarea = textareaRef.current;
      textarea?.focus();
      textarea?.setSelectionRange(textarea.value.length, textarea.value.length);
    }
  }, [editing, readOnly, selected]);

  /** Enters text editing after React Flow has resolved the gesture as a click. */
  function startEditing(): void {
    if (!readOnly) {
      setEditing(true);
    }
  }

  return (
    <div
      className={`relative size-full min-h-24 min-w-48 rounded-xl border p-3 shadow-sm ${!readOnly && !(editing && selected) ? "cursor-grab active:cursor-grabbing" : ""} ${THEME_CLASSES[data.theme]}`}
      data-workflow-annotation-id={id}
    >
      <NodeResizer
        isVisible={selected && !readOnly}
        minWidth={192}
        minHeight={96}
        lineClassName="border-ring"
        handleClassName="size-2.5! border-ring! bg-background!"
      />
      {editing && selected && !readOnly ? (
        <textarea
          ref={textareaRef}
          className="nodrag nopan nowheel size-full resize-none bg-transparent text-sm leading-5 outline-none placeholder:text-current/45"
          value={data.text}
          aria-label={t("settings.workflow.annotationText")}
          placeholder={t("settings.workflow.annotationPlaceholder")}
          onBlur={() => setEditing(false)}
          onChange={(event) => actions.update(id, { text: event.target.value })}
          onKeyDown={(event) => {
            event.stopPropagation();
            if (event.key === "Escape") {
              event.preventDefault();
              setEditing(false);
            }
          }}
        />
      ) : (
        <div
          className="size-full select-none whitespace-pre-wrap break-words text-sm leading-5"
          aria-label={t("settings.workflow.annotationText")}
          title={
            readOnly ? undefined : t("settings.workflow.annotationEditHint")
          }
          data-annotation-editing="false"
          onClick={() => {
            startEditing();
          }}
        >
          {data.text === "" ? (
            <span className="text-current/45">
              {t("settings.workflow.annotationPlaceholder")}
            </span>
          ) : (
            data.text
          )}
        </div>
      )}
      {selected && !readOnly && (
        <div
          className="nodrag absolute -bottom-9 left-0 flex h-8 items-center gap-1 rounded-lg border border-border bg-background px-1 shadow-sm"
          onPointerDown={(event) => event.preventDefault()}
        >
          {WORKFLOW_ANNOTATION_THEMES.map((theme) => (
            <button
              key={theme}
              type="button"
              className={`size-5 rounded-full border ${THEME_CLASSES[theme]} ${theme === data.theme ? "ring-2 ring-ring ring-offset-1" : ""}`}
              aria-label={t("settings.workflow.annotationTheme", { theme })}
              aria-pressed={theme === data.theme}
              onClick={() => actions.update(id, { theme })}
            />
          ))}
          <button
            type="button"
            className="ml-1 flex size-6 items-center justify-center rounded-md text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
            aria-label={t("settings.workflow.deleteAnnotation")}
            onClick={() => actions.remove(id)}
          >
            <IconTrash className="size-4" />
          </button>
        </div>
      )}
    </div>
  );
}
