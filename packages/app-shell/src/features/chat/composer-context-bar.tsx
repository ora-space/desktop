import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@ora/ui";
import {
  IconCheck,
  IconChevronRight,
  IconCloud,
  IconDeviceLaptop,
  IconFolder,
  IconGitBranch,
  IconPlus,
} from "@tabler/icons-react";
import { useProjects } from "../../state/hooks/use-projects";
import { useTasks } from "../../state/hooks/use-tasks";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";

/**
 * The strip above the composer that states which project, environment, and
 * branch a new task will run against.
 *
 * Project and branch are wired to the workspace selection. The environment tab
 * picks a value but has no execution behind it yet.
 */
export function ComposerContextBar() {
  return (
    // Bottom padding runs under the composer card, which is what makes the two
    // read as one stacked surface instead of two separate controls.
    <div className="flex items-center gap-0.5 rounded-t-xl bg-muted px-1.5 pb-4 pt-1">
      <ProjectTab />
      <EnvironmentTab />
      <BranchTab />
    </div>
  );
}

/** Where a task runs. Local is the only environment the runtime can service today. */
type TaskEnvironment = "local" | "cloud";

/**
 * Chooses between local and cloud execution.
 *
 * The choice is component state on purpose: nothing consumes it yet, so lifting
 * it into a store would only invent a contract that the runtime has to match
 * later. It moves once one of the two environments actually dispatches work.
 */
function EnvironmentTab() {
  const { t } = useTranslation();
  const [environment, setEnvironment] = useState<TaskEnvironment>("local");

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className={CONTEXT_TAB_CLASS}
            aria-label={t("chat.contextBar.selectEnvironment")}
          />
        }
      >
        {environment === "local" ? <IconDeviceLaptop className="size-3.5" /> : <IconCloud className="size-3.5" />}
        {environment === "local" ? t("chat.local") : t("chat.cloud")}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" side="top" className={MENU_WIDTH_CLASS}>
        {/* DropdownMenuLabel is a group label, so it throws unless a group owns it. */}
        <DropdownMenuGroup className={MENU_GROUP_CLASS}>
          <DropdownMenuLabel className={MENU_LABEL_CLASS}>{t("chat.contextBar.launchMode")}</DropdownMenuLabel>
          <DropdownMenuItem className={MENU_ITEM_CLASS} onClick={() => setEnvironment("local")}>
            <IconDeviceLaptop className={MENU_ICON_CLASS} />
            {t("chat.contextBar.runLocally")}
            {environment === "local" && <IconCheck className={MENU_CHECK_CLASS} />}
          </DropdownMenuItem>
          <DropdownMenuItem className={MENU_ITEM_CLASS} onClick={() => setEnvironment("cloud")}>
            <IconCloud className={MENU_ICON_CLASS} />
            {t("chat.cloud")}
            {environment === "cloud" && <IconCheck className={MENU_CHECK_CLASS} />}
          </DropdownMenuItem>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** Shared trigger styling so the live project tab and the inert tabs stay on one baseline. */
const CONTEXT_TAB_CLASS = "h-6 gap-1.5 px-2 text-xs font-normal text-muted-foreground";

/**
 * Metrics shared by every context-bar menu.
 *
 * The project picker is a Command popup (it needs search) while the environment
 * picker is a plain menu, and the two primitives ship different padding, radius,
 * and hover colours. These constants are what keep them looking like one family;
 * any tab added later should style itself from here rather than from the
 * primitive's defaults.
 */
const MENU_WIDTH_CLASS = "w-52";
/** Command nests a group inside its root, so plain menus need the same second inset. */
const MENU_GROUP_CLASS = "p-1 **:[[cmdk-group-heading]]:font-normal";
/**
 * `text-foreground` is deliberate: menu popups default to `--popover-foreground`,
 * which is a shade darker than the `--foreground` that CommandGroup sets, so
 * without it the two menus render their labels at different weights.
 */
const MENU_ITEM_CLASS = "gap-1.5 rounded-sm px-2 py-1.5 text-xs text-foreground focus:bg-muted focus:text-foreground";
const MENU_LABEL_CLASS = "px-2 py-1.5 text-xs font-normal text-muted-foreground";
/** Leading icons stay muted so the label carries the row; only the trailing check is full strength. */
const MENU_ICON_CLASS = "size-3.5 text-muted-foreground";
/** Command's built-in trailing check renders at size-4; hand-rolled ones must match. */
const MENU_CHECK_CLASS = "ml-auto size-4";

/** Selects the project a new task belongs to, or creates one through the shared workspace dialog. */
function ProjectTab() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const { data: projects = [] } = useProjects();
  const selectedProjectId = useWorkspaceSelectionStore((s) => s.selection.projectId);
  const selectProject = useWorkspaceSelectionStore((s) => s.selectProject);
  const setDialog = useUiStore((s) => s.setDialog);

  const selectedProject = projects.find((project) => project.id === selectedProjectId);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className={CONTEXT_TAB_CLASS}
            aria-label={t("chat.contextBar.selectProject")}
          />
        }
      >
        <IconFolder className="size-3.5" />
        <span className="max-w-40 truncate">{selectedProject?.name ?? t("chat.contextBar.noProject")}</span>
      </PopoverTrigger>
      <PopoverContent align="start" side="top" className={`${MENU_WIDTH_CLASS} p-0`}>
        <Command>
          <CommandInput placeholder={t("chat.contextBar.searchProjects")} className="text-xs" />
          <CommandList>
            <CommandEmpty className="py-4 text-xs">{t("chat.contextBar.noProjectsFound")}</CommandEmpty>
            <CommandGroup className={MENU_GROUP_CLASS}>
              {projects.map((project) => (
                // `data-checked` drives CommandItem's own trailing check. Rendering a
                // second `ml-auto` icon here instead would fight that built-in one and
                // pull both off the right edge.
                <CommandItem
                  key={project.id}
                  value={project.name}
                  data-checked={project.id === selectedProjectId}
                  className={MENU_ITEM_CLASS}
                  onSelect={() => {
                    selectProject(project.id);
                    setOpen(false);
                  }}
                >
                  <IconFolder className={MENU_ICON_CLASS} />
                  <span className="truncate">{project.name}</span>
                </CommandItem>
              ))}
            </CommandGroup>
            <CommandSeparator />
            <CommandGroup className={MENU_GROUP_CLASS}>
              {/* Reuses the sidebar's project form verbatim; its mutation selects the
                  new project, which is what feeds the label back into this tab. */}
              <CommandItem
                value={t("sidebar.newProject")}
                className={MENU_ITEM_CLASS}
                onSelect={() => {
                  setOpen(false);
                  setDialog({ kind: "project" });
                }}
              >
                <IconPlus className={MENU_ICON_CLASS} />
                {t("sidebar.newProject")}
                {/* CommandShortcut both right-aligns the chevron and suppresses the
                    built-in check, so this row lines up with the project rows above. */}
                <CommandShortcut>
                  <IconChevronRight className="size-3.5" />
                </CommandShortcut>
              </CommandItem>
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

/**
 * Selects which worktree a task runs in, or creates a new one.
 *
 * Tasks stand in for branches here: the backend owns the worktree and its branch
 * name and does not expose either through the frontend contract, so the task
 * title is the only handle the UI has. The secondary line shows task status for
 * the same reason — working-tree dirtiness is not something we can ask for yet.
 */
function BranchTab() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const { data: tasks = [] } = useTasks();
  const selection = useWorkspaceSelectionStore((s) => s.selection);
  const selectTask = useWorkspaceSelectionStore((s) => s.selectTask);
  const setDialog = useUiStore((s) => s.setDialog);

  const projectId = selection.projectId;
  const projectTasks = tasks.filter((task) => task.projectId === projectId);
  const selectedTask = tasks.find((task) => task.id === selection.taskId);

  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className={CONTEXT_TAB_CLASS}
            aria-label={t("chat.contextBar.selectBranch")}
          />
        }
      >
        <IconGitBranch className="size-3.5" />
        <span className="max-w-40 truncate">{selectedTask?.title ?? t("chat.contextBar.defaultBranch")}</span>
      </PopoverTrigger>
      <PopoverContent align="start" side="top" className={`${MENU_WIDTH_CLASS} p-0`}>
        <Command>
          <CommandInput placeholder={t("chat.contextBar.searchBranches")} className="text-xs" />
          <CommandList>
            <CommandEmpty className="py-4 text-xs">{t("chat.contextBar.noBranchesFound")}</CommandEmpty>
            <CommandGroup heading={t("chat.contextBar.branches")} className={MENU_GROUP_CLASS}>
              {projectTasks.map((task) => (
                <CommandItem
                  key={task.id}
                  value={task.title}
                  data-checked={task.id === selection.taskId}
                  className={MENU_ITEM_CLASS}
                  onSelect={() => {
                    selectTask(task.id, task.projectId);
                    setOpen(false);
                  }}
                >
                  <IconGitBranch className={MENU_ICON_CLASS} />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate">{task.title}</span>
                    <span className="block truncate text-[10px] text-muted-foreground">{t(`common.${task.status}`)}</span>
                  </span>
                </CommandItem>
              ))}
            </CommandGroup>
            <CommandSeparator />
            <CommandGroup className={MENU_GROUP_CLASS}>
              {/* Same ui-store dialog the sidebar's "new worktree" action opens, so
                  both paths share one form and one create mutation. */}
              <CommandItem
                value={t("chat.contextBar.createBranch")}
                className={MENU_ITEM_CLASS}
                disabled={projectId === null}
                onSelect={() => {
                  if (projectId === null) return;
                  setOpen(false);
                  setDialog({ kind: "task", projectId });
                }}
              >
                <IconPlus className={MENU_ICON_CLASS} />
                {t("chat.contextBar.createBranch")}
              </CommandItem>
            </CommandGroup>
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

