import { useMemo, useRef, useState, type ChangeEvent } from "react";
import { useTranslation } from "react-i18next";
import {
  IconFileImport,
  IconLayoutSidebarLeftCollapse,
  IconPencil,
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
import type { DemoWorkflow } from "@ora/workflow-mock";

interface WorkflowManagerProps {
  workflows: DemoWorkflow[];
  selectedWorkflowId: string | null;
  error: string | null;
  onSelect: (workflowId: string) => void;
  onCreate: (name: string) => void;
  onRename: (workflowId: string, name: string) => void;
  onDelete: (workflowId: string) => void;
  onImport: (file: File) => void;
  onCollapse: () => void;
}

/** Keeps workflow-level actions separate from graph construction controls. */
export function WorkflowManager({
  workflows,
  selectedWorkflowId,
  error,
  onSelect,
  onCreate,
  onRename,
  onDelete,
  onImport,
  onCollapse,
}: WorkflowManagerProps) {
  const { i18n, t } = useTranslation();
  const [query, setQuery] = useState("");
  const [newWorkflowName, setNewWorkflowName] = useState("");
  const [renameWorkflowName, setRenameWorkflowName] = useState("");
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [renameTarget, setRenameTarget] = useState<DemoWorkflow | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<DemoWorkflow | null>(null);
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

  /** Opens workflow creation with an empty name so the user must choose one. */
  function openCreateDialog(): void {
    setNewWorkflowName("");
    setCreateDialogOpen(true);
  }

  /** Creates a workflow only when the submitted name remains non-empty after trimming. */
  function submitCreateWorkflow(): void {
    const name = newWorkflowName.trim();
    if (name === "") {
      return;
    }
    onCreate(name);
    setCreateDialogOpen(false);
  }

  /** Opens workflow rename with the current name so edits are incremental. */
  function openRenameDialog(workflow: DemoWorkflow): void {
    setRenameWorkflowName(workflow.name);
    setRenameTarget(workflow);
  }

  /** Renames the selected session entry using its stable identity. */
  function submitRenameWorkflow(): void {
    if (renameTarget === null) {
      return;
    }
    const name = renameWorkflowName.trim();
    if (name === "") {
      return;
    }
    onRename(renameTarget.id, name);
    setRenameTarget(null);
  }

  return (
    <aside className="flex min-h-0 flex-1 flex-col border-r border-border bg-background">
      <div className="space-y-3 border-b border-border p-3">
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0">
            <h3 className="text-xs font-semibold">{t("settings.workflow.library")}</h3>
            <p className="mt-0.5 text-[10px] text-muted-foreground">
              {t("settings.workflow.workflowCount", { count: workflows.length })}
            </p>
          </div>
          <div className="flex items-center gap-1">
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("settings.workflow.collapseLibrary")}
              onClick={onCollapse}
            >
              <IconLayoutSidebarLeftCollapse />
            </Button>
            <Button
              size="icon-sm"
              aria-label={t("settings.workflow.newWorkflow")}
              onClick={openCreateDialog}
            >
              <IconPlus />
            </Button>
          </div>
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
                aria-label={t("settings.workflow.renameNamed", { name: workflow.name })}
                onClick={() => openRenameDialog(workflow)}
                className={cn(
                  "absolute right-8 top-1.5 flex size-7 items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring",
                  selected ? "opacity-100" : "opacity-0 group-hover:opacity-100 group-focus-within:opacity-100",
                )}
              >
                <IconPencil className="size-3.5" />
              </button>
              <button
                type="button"
                aria-label={t("settings.workflow.deleteNamed", { name: workflow.name })}
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
        open={createDialogOpen}
        onOpenChange={(open) => {
          if (!open) {
            setCreateDialogOpen(false);
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("settings.workflow.createWorkflowTitle")}</AlertDialogTitle>
          </AlertDialogHeader>
          <Input
            value={newWorkflowName}
            onChange={(event) => setNewWorkflowName(event.target.value)}
            aria-label={t("settings.workflow.workflowName")}
            placeholder={t("settings.workflow.workflowNamePlaceholder")}
            autoFocus
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                submitCreateWorkflow();
              }
            }}
          />
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={newWorkflowName.trim() === ""}
              onClick={submitCreateWorkflow}
            >
              {t("settings.workflow.newWorkflow")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog
        open={renameTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
            setRenameTarget(null);
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.workflow.renameWorkflowTitle", { name: renameTarget?.name ?? "" })}
            </AlertDialogTitle>
          </AlertDialogHeader>
          <Input
            value={renameWorkflowName}
            onChange={(event) => setRenameWorkflowName(event.target.value)}
            aria-label={t("settings.workflow.workflowName")}
            placeholder={t("settings.workflow.workflowNamePlaceholder")}
            autoFocus
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                submitRenameWorkflow();
              }
            }}
          />
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={renameWorkflowName.trim() === ""}
              onClick={submitRenameWorkflow}
            >
              {t("settings.workflow.renameWorkflow")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => {
          if (!open) {
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
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
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
