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
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@ora/ui";
import {
  IconChevronRight,
  IconDeviceLaptop,
  IconFolder,
  IconGitBranch,
  IconPlus,
} from "@tabler/icons-react";
import { useProjects } from "../../state/hooks/use-projects";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";

/**
 * The strip above the composer that states which project, environment, and
 * branch a new task will run against.
 *
 * Only the project tab is wired up; environment and branch render as inert
 * placeholders so the row's final shape is visible while their backing
 * selections are still being designed.
 */
export function ComposerContextBar() {
  const { t } = useTranslation();

  return (
    // Bottom padding runs under the composer card, which is what makes the two
    // read as one stacked surface instead of two separate controls.
    <div className="flex items-center gap-0.5 rounded-t-xl bg-muted px-1.5 pb-4 pt-1">
      <ProjectTab />
      <ContextTabPlaceholder icon={<IconDeviceLaptop className="size-3.5" />} label={t("chat.local")} />
      <ContextTabPlaceholder icon={<IconGitBranch className="size-3.5" />} label={t("chat.contextBar.defaultBranch")} />
    </div>
  );
}

/** Shared trigger styling so the live project tab and the inert tabs stay on one baseline. */
const CONTEXT_TAB_CLASS = "h-6 gap-1.5 px-2 text-xs font-normal text-muted-foreground";

/** Keeps the picker's type at the same size as the tab that opens it. */
const MENU_TEXT_CLASS = "text-xs";

/** Menu rows also tighten their icon slot so labels align with the tab row above. */
const MENU_ITEM_CLASS = `${MENU_TEXT_CLASS} gap-1.5`;

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
      <PopoverContent align="start" side="top" className="w-52 p-0">
        <Command>
          <CommandInput placeholder={t("chat.contextBar.searchProjects")} className={MENU_TEXT_CLASS} />
          <CommandList>
            <CommandEmpty className={`py-4 ${MENU_TEXT_CLASS}`}>{t("chat.contextBar.noProjectsFound")}</CommandEmpty>
            <CommandGroup>
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
                  <IconFolder className="size-3.5 text-muted-foreground" />
                  <span className="truncate">{project.name}</span>
                </CommandItem>
              ))}
            </CommandGroup>
            <CommandSeparator />
            <CommandGroup>
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
                <IconPlus className="size-3.5 text-muted-foreground" />
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

/** Holds a tab's place in the row until its selection model exists. */
function ContextTabPlaceholder({ icon, label }: { icon: React.ReactNode; label: string }) {
  return (
    <Button type="button" variant="ghost" size="sm" disabled className={CONTEXT_TAB_CLASS}>
      {icon}
      {label}
    </Button>
  );
}
