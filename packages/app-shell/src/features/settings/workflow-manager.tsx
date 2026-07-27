import { useMemo, useRef, useState, type ChangeEvent } from "react";
import { useTranslation } from "react-i18next";
import {
  IconFileImport,
  IconPlus,
  IconRoute,
  IconSearch,
  IconTrash,
} from "@tabler/icons-react";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  Input,
  cn,
} from "@ora/ui";
import type { WorkflowDefinition } from "@ora/workflow-mock";

interface WorkflowManagerProps {
  workflows: WorkflowDefinition[];
  selectedWorkflowId: string | null;
  busy: boolean;
  error: string | null;
  onSelect: (workflowId: string) => void;
  onCreate: () => void;
  onDelete: (workflowId: string) => void;
  onImport: (file: File) => void;
}

/** Keeps workflow-level actions separate from graph construction controls. */
export function WorkflowManager({
  workflows,
  selectedWorkflowId,
  busy,
  error,
  onSelect,
  onCreate,
  onDelete,
  onImport,
}: WorkflowManagerProps) {
  const { i18n, t } = useTranslation();
  const [query, setQuery] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<WorkflowDefinition | null>(null);
  const importInputRef = useRef<HTMLInputElement>(null);
  const visibleWorkflows = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase();
    if (normalizedQuery === "") {
      return workflows;
    }
    return workflows.filter((workflow) =>
      `${workflow.name} ${workflow.description}`.toLocaleLowerCase().includes(normalizedQuery),
    );
  }, [query, workflows]);

  /** Forwards one selected JSON file and clears the native input so it can be chosen again. */
  function handleImport(event: ChangeEvent<HTMLInputElement>): void {
    const [file] = Array.from(event.target.files ?? []);
    if (file !== undefined) {
      onImport(file);
    }
    event.target.value = "";
  }

  return (
    <aside className="flex min-h-0 flex-col border-r border-border bg-background">
      <div className="space-y-3 border-b border-border p-3">
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0">
            <h3 className="text-xs font-semibold">{t("settings.workflow.library")}</h3>
            <p className="mt-0.5 text-[10px] text-muted-foreground">
              {t("settings.workflow.workflowCount", { count: workflows.length })}
            </p>
          </div>
          <Button
            size="icon-sm"
            aria-label={t("settings.workflow.newWorkflow")}
            disabled={busy}
            onClick={onCreate}
          >
            <IconPlus />
          </Button>
        </div>
        <div className="relative">
          <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            aria-label={t("settings.workflow.searchWorkflows")}
            placeholder={t("settings.workflow.searchWorkflows")}
            className="h-8 pl-8 text-xs"
          />
        </div>
      </div>
      <div className="min-h-0 flex-1 space-y-1 overflow-y-auto p-2">
        {visibleWorkflows.map((workflow) => {
          const selected = workflow.id === selectedWorkflowId;
          return (
            <div
              key={workflow.id}
              className={cn(
                "group relative rounded-lg border transition-colors",
                selected
                  ? "border-foreground/20 bg-muted/80 shadow-sm"
                  : "border-transparent hover:border-border hover:bg-muted/45",
              )}
            >
              <button
                type="button"
                onClick={() => onSelect(workflow.id)}
                className="flex min-h-14 w-full items-start gap-2.5 rounded-lg px-2.5 py-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
              >
                <span
                  className={cn(
                    "mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-md",
                    selected ? "bg-foreground text-background" : "bg-muted text-muted-foreground",
                  )}
                >
                  <IconRoute className="size-3.5" />
                </span>
                <span className="min-w-0 flex-1 pr-6">
                  <span className="block truncate text-[11px] font-medium">{workflow.name}</span>
                  <span className="mt-0.5 block truncate text-[9px] text-muted-foreground">
                    {new Intl.DateTimeFormat(i18n.resolvedLanguage, {
                      month: "short",
                      day: "numeric",
                    }).format(new Date(workflow.updatedAt))}
                  </span>
                </span>
              </button>
              <button
                type="button"
                aria-label={t("settings.workflow.deleteNamed", { name: workflow.name })}
                disabled={busy}
                onClick={() => setDeleteTarget(workflow)}
                className={cn(
                  "absolute right-1.5 top-1.5 flex size-7 items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-destructive/10 hover:text-destructive focus-visible:ring-2 focus-visible:ring-ring",
                  selected ? "opacity-100" : "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100",
                )}
              >
                <IconTrash className="size-3.5" />
              </button>
            </div>
          );
        })}
        {visibleWorkflows.length === 0 && (
          <p className="px-2 py-8 text-center text-[11px] text-muted-foreground">
            {t("settings.workflow.noWorkflows")}
          </p>
        )}
      </div>
      <div className="border-t border-border p-3">
        {error !== null && (
          <p role="alert" className="mb-2 text-[10px] leading-4 text-destructive">
            {error}
          </p>
        )}
        <input
          ref={importInputRef}
          type="file"
          accept=".json,application/json"
          className="hidden"
          onChange={handleImport}
        />
        <Button
          variant="outline"
          size="sm"
          className="w-full justify-start"
          disabled={busy}
          onClick={() => importInputRef.current?.click()}
        >
          <IconFileImport />
          {t("settings.workflow.importWorkflow")}
        </Button>
        <p className="mt-1.5 text-[9px] leading-3 text-muted-foreground">
          {t("settings.workflow.importHint")}
        </p>
      </div>
      <AlertDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open && !busy) {
            setDeleteTarget(null);
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.workflow.deleteWorkflowTitle", { name: deleteTarget?.name ?? "" })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.workflow.deleteWorkflowDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={busy}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={busy}
              onClick={() => {
                if (deleteTarget !== null) {
                  onDelete(deleteTarget.id);
                  setDeleteTarget(null);
                }
              }}
            >
              <IconTrash />
              {t("common.delete")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </aside>
  );
}
