import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  IconCopy,
  IconDotsVertical,
  IconDownload,
  IconFileImport,
  IconPencil,
  IconPlus,
  IconRoute,
  IconSearch,
  IconTrash,
  IconX,
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
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
  Input,
  cn,
} from "@ora/ui";
/** Sidebar row identity; the canvas hydrates the full graph separately. */
export interface WorkflowLibraryItem {
  id: string;
  name: string;
  /** Imported during this session; marked so users can find the new row. */
  imported?: boolean;
}

interface WorkflowManagerProps {
  workflows: WorkflowLibraryItem[];
  /** False until the library has loaded, so a pending or failed load never reads as empty. */
  libraryLoaded: boolean;
  selectedWorkflowId: string | null;
  error: string | null;
  /** True until the open editor has registered flush-before-switch actions. */
  disabled?: boolean;
  /** Failed editors permit another selection while mutation actions remain unavailable. */
  selectionDisabled?: boolean;
  onSelect: (workflowId: string) => void;
  onCreate: (name: string) => Promise<boolean>;
  onCopy: (workflowId: string) => Promise<boolean>;
  onRename: (workflowId: string, name: string) => Promise<boolean>;
  onDelete: (workflowId: string) => void;
  onImport: () => void;
  onExport: (workflowId: string) => void;
}

/** Keeps workflow-level actions in the app sidebar, separate from graph construction. */
export function WorkflowManager({
  workflows,
  libraryLoaded,
  selectedWorkflowId,
  error,
  disabled = false,
  selectionDisabled = disabled,
  onSelect,
  onCreate,
  onCopy,
  onRename,
  onDelete,
  onImport,
  onExport,
}: WorkflowManagerProps) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [newWorkflowName, setNewWorkflowName] = useState("");
  const [renameWorkflowName, setRenameWorkflowName] = useState("");
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [renameTarget, setRenameTarget] = useState<WorkflowLibraryItem | null>(
    null,
  );
  const [deleteTarget, setDeleteTarget] = useState<WorkflowLibraryItem | null>(
    null,
  );
  const [createBusy, setCreateBusy] = useState(false);
  const [renameBusy, setRenameBusy] = useState(false);
  const [copyBusy, setCopyBusy] = useState(false);
  const visibleWorkflows = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase();
    if (normalizedQuery === "") {
      return workflows;
    }
    return workflows.filter((workflow) =>
      workflow.name.toLocaleLowerCase().includes(normalizedQuery),
    );
  }, [query, workflows]);

  /** Opens workflow creation with an empty name so the user must choose one. */
  function openCreateDialog(): void {
    if (disabled) {
      return;
    }
    setNewWorkflowName("");
    setCreateDialogOpen(true);
  }

  useEffect(() => {
    function handleNewWorkflowShortcut(event: KeyboardEvent): void {
      if (
        !(event.metaKey || event.ctrlKey) ||
        event.key.toLowerCase() !== "n" ||
        event.repeat
      ) {
        return;
      }
      // Match the sidebar's Ctrl/Cmd+N intercept so the browser does not open a
      // window, then open create instead of starting a chat.
      event.preventDefault();
      if (disabled || createDialogOpen || createBusy) {
        return;
      }
      setNewWorkflowName("");
      setCreateDialogOpen(true);
    }
    window.addEventListener("keydown", handleNewWorkflowShortcut);
    return () => {
      window.removeEventListener("keydown", handleNewWorkflowShortcut);
    };
  }, [createBusy, createDialogOpen, disabled]);

  /** Creates a workflow only when the submitted name remains non-empty after trimming. */
  async function submitCreateWorkflow(): Promise<void> {
    const name = newWorkflowName.trim();
    if (name === "" || createBusy) {
      return;
    }
    setCreateBusy(true);
    try {
      const created = await onCreate(name);
      if (created) {
        setQuery("");
        setCreateDialogOpen(false);
      }
    } finally {
      setCreateBusy(false);
    }
  }

  /** Opens workflow rename with the current name so edits are incremental. */
  function openRenameDialog(workflow: WorkflowLibraryItem): void {
    if (disabled) {
      return;
    }
    setRenameWorkflowName(workflow.name);
    setRenameTarget(workflow);
  }

  /** Duplicates the library entry through the registered flush-and-switch action. */
  async function copyWorkflow(workflowId: string): Promise<void> {
    if (disabled || copyBusy) {
      return;
    }
    setCopyBusy(true);
    try {
      await onCopy(workflowId);
    } finally {
      setCopyBusy(false);
    }
  }

  /** Renames the selected library entry using its stable identity. */
  async function submitRenameWorkflow(): Promise<void> {
    if (renameTarget === null || renameBusy) {
      return;
    }
    const name = renameWorkflowName.trim();
    if (name === "") {
      return;
    }
    setRenameBusy(true);
    try {
      const renamed = await onRename(renameTarget.id, name);
      if (renamed) {
        setQuery("");
        setRenameTarget(null);
      }
    } finally {
      setRenameBusy(false);
    }
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <div className="px-2 pb-3">
        <div className="relative min-w-0">
          <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            aria-label={t("settings.workflow.searchWorkflows")}
            placeholder={t("settings.workflow.searchWorkflows")}
            className="h-9 border-transparent bg-sidebar-accent/60 px-8 text-[13px] shadow-none hover:bg-sidebar-accent focus-visible:bg-background"
          />
          {query && (
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="absolute right-1 top-1/2 -translate-y-1/2"
              aria-label={t("sidebar.clearSearch")}
              onClick={() => setQuery("")}
            >
              <IconX />
            </Button>
          )}
        </div>
      </div>
      <div className="flex h-8 items-center pl-4 pr-3 text-xs font-medium text-muted-foreground">
        <span>{t("sidebar.workflows")}</span>
        {/* Adding a workflow is one intent with two sources, so create and import share
            the + menu; Ctrl/Cmd+N still opens create directly without the menu. */}
        <DropdownMenu>
          <DropdownMenuTrigger
            disabled={disabled}
            aria-label={t("settings.workflow.transfer.addMenu")}
            className="ml-auto flex size-7 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-sidebar-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-popup-open:bg-sidebar-accent data-popup-open:text-foreground disabled:opacity-60"
          >
            <IconPlus className="size-4" />
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-48">
            <DropdownMenuItem disabled={disabled} onClick={openCreateDialog}>
              <IconPlus className="size-3.5" />
              {t("settings.workflow.newWorkflow")}
              <DropdownMenuShortcut>
                {t("settings.workflow.transfer.newWorkflowShortcut")}
              </DropdownMenuShortcut>
            </DropdownMenuItem>
            <DropdownMenuItem disabled={disabled} onClick={onImport}>
              <IconFileImport className="size-3.5" />
              {t("settings.workflow.transfer.importMenuItem")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      {error !== null && (
        <p
          role="alert"
          className="mx-2 mb-2 rounded-md bg-destructive/10 px-2 py-1.5 text-[11px] leading-4 text-destructive"
        >
          {error}
        </p>
      )}
      <div className="min-h-0 min-w-0 flex-1 space-y-0.5 overflow-y-auto px-2 pb-3">
        {visibleWorkflows.map((workflow) => {
          const selected = workflow.id === selectedWorkflowId;
          return (
            <div
              key={workflow.id}
              className={cn(
                "group/tree flex h-9 items-center rounded-md transition-colors",
                selected
                  ? "bg-sidebar-accent text-sidebar-accent-foreground"
                  : "hover:bg-sidebar-accent/70",
              )}
            >
              <button
                type="button"
                disabled={selectionDisabled}
                onClick={() => onSelect(workflow.id)}
                className="flex h-full min-w-0 flex-1 cursor-pointer items-center gap-2 rounded-md px-2 text-left text-[13px] outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:cursor-default disabled:opacity-60"
              >
                <IconRoute className="size-4 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate font-medium">
                  {workflow.name}
                </span>
                {workflow.imported === true && (
                  <span className="shrink-0 rounded-full bg-blue-500/10 px-1.5 py-px text-[10px] font-medium text-blue-700 dark:text-blue-300">
                    {t("settings.workflow.transfer.importedBadge")}
                  </span>
                )}
              </button>
              <DropdownMenu>
                <DropdownMenuTrigger
                  disabled={disabled}
                  aria-label={t("settings.workflow.openActions", {
                    name: workflow.name,
                  })}
                  className={cn(
                    "mr-1 flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-sidebar-accent/70 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-popup-open:opacity-100 disabled:opacity-60",
                    selected
                      ? "opacity-100"
                      : "opacity-0 group-hover/tree:opacity-100 group-focus-within/tree:opacity-100",
                  )}
                >
                  <IconDotsVertical className="size-4" />
                </DropdownMenuTrigger>
                <DropdownMenuContent align="end" className="w-36">
                  <DropdownMenuItem
                    disabled={disabled || copyBusy}
                    onClick={() => void copyWorkflow(workflow.id)}
                  >
                    <IconCopy className="size-3.5" />
                    {t("settings.workflow.copyWorkflow")}
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    disabled={disabled}
                    onClick={() => openRenameDialog(workflow)}
                  >
                    <IconPencil className="size-3.5" />
                    {t("settings.workflow.renameWorkflow")}
                  </DropdownMenuItem>
                  <DropdownMenuItem
                    disabled={disabled}
                    onClick={() => onExport(workflow.id)}
                  >
                    <IconDownload className="size-3.5" />
                    {t("settings.workflow.transfer.exportMenuItem")}
                  </DropdownMenuItem>
                  <DropdownMenuSeparator />
                  <DropdownMenuItem
                    disabled={disabled}
                    variant="destructive"
                    onClick={() => setDeleteTarget(workflow)}
                  >
                    <IconTrash className="size-3.5" />
                    {t("common.delete")}
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
            </div>
          );
        })}
        {visibleWorkflows.length === 0 &&
          libraryLoaded &&
          (workflows.length === 0 ? (
            <div className="grid justify-items-center gap-2.5 px-2 py-7 text-center">
              <p className="text-[13px] text-muted-foreground">
                {t("settings.workflow.emptyTitle")}
              </p>
              <div className="flex flex-wrap justify-center gap-1.5">
                <Button
                  variant="outline"
                  size="sm"
                  disabled={disabled}
                  onClick={openCreateDialog}
                >
                  <IconPlus />
                  {t("settings.workflow.transfer.emptyCreate")}
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={disabled}
                  onClick={onImport}
                >
                  <IconFileImport />
                  {t("settings.workflow.transfer.emptyImport")}
                </Button>
              </div>
            </div>
          ) : (
            <p className="px-2 py-6 text-center text-[13px] text-muted-foreground">
              {t("settings.workflow.noWorkflows")}
            </p>
          ))}
      </div>
      <AlertDialog
        open={createDialogOpen}
        onOpenChange={(open) => {
          if (!open && !createBusy) {
            setCreateDialogOpen(false);
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.workflow.createWorkflowTitle")}
            </AlertDialogTitle>
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
                void submitCreateWorkflow();
              }
            }}
          />
          {error !== null && (
            <p role="alert" className="text-[13px] text-destructive">
              {error}
            </p>
          )}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={createBusy}>
              {t("common.cancel")}
            </AlertDialogCancel>
            <AlertDialogAction
              disabled={newWorkflowName.trim() === "" || createBusy}
              onClick={(event) => {
                event.preventDefault();
                void submitCreateWorkflow();
              }}
            >
              {t("settings.workflow.newWorkflow")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog
        open={renameTarget !== null}
        onOpenChange={(open) => {
          if (!open && !renameBusy) {
            setRenameTarget(null);
          }
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.workflow.renameWorkflowTitle", {
                name: renameTarget?.name ?? "",
              })}
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
                void submitRenameWorkflow();
              }
            }}
          />
          {error !== null && (
            <p role="alert" className="text-[13px] text-destructive">
              {error}
            </p>
          )}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={renameBusy}>
              {t("common.cancel")}
            </AlertDialogCancel>
            <AlertDialogAction
              disabled={renameWorkflowName.trim() === "" || renameBusy}
              onClick={(event) => {
                event.preventDefault();
                void submitRenameWorkflow();
              }}
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
              {t("settings.workflow.deleteWorkflowTitle", {
                name: deleteTarget?.name ?? "",
              })}
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
    </div>
  );
}
