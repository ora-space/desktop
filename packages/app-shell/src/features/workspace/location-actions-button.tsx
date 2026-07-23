import { useTranslation } from "react-i18next";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
  cn,
  toast,
} from "@ora/ui";
import {
  IconBrandVscode,
  IconCheck,
  IconChevronDown,
  IconCopy,
  IconFolder,
  IconTerminal2,
} from "@tabler/icons-react";
import type { ComponentType } from "react";
import { usePlatform } from "@ora/platform";
import type { LocationTarget } from "@ora/platform";
import {
  useLocationActionsStore,
  type DefaultLocationTarget,
} from "../../state/stores/location-actions-store";

/** The openers offered in the menu, in display order. Copy Path is handled separately. */
const OPENERS: readonly DefaultLocationTarget[] = ["explorer", "terminal", "vscode"] as const;

const OPENER_ICONS: Record<DefaultLocationTarget, ComponentType<{ className?: string }>> = {
  explorer: IconFolder,
  terminal: IconTerminal2,
  vscode: IconBrandVscode,
};

const OPENER_LABEL_KEYS: Record<DefaultLocationTarget, string> = {
  explorer: "locationActions.explorer",
  terminal: "locationActions.terminal",
  vscode: "locationActions.vscode",
};

interface LocationActionsButtonProps {
  /**
   * The task whose git worktree the actions target. When present it wins over
   * `projectPath`, so an open acts on the worktree the user is actually looking at.
   */
  taskId?: string | null;
  /** The project root used when no task is in context (worktree resolution is skipped). */
  projectPath?: string | null;
}

/**
 * A split button in the window's top-right toolbar that opens the current context's
 * directory in the file manager, a terminal, or VS Code - or copies its path.
 *
 * The icon half repeats the remembered default opener (its glyph follows that default);
 * the chevron half opens a menu to switch it. Choosing an opener both runs it and makes
 * it the new default. "Copy Path" is a menu-only action that never becomes the default.
 *
 * Desktop-only: on the Web host `locationActions` is unsupported and this renders nothing.
 */
export function LocationActionsButton({ taskId, projectPath }: LocationActionsButtonProps) {
  const { t } = useTranslation();
  const { locationActions } = usePlatform();
  const defaultTarget = useLocationActionsStore((state) => state.defaultTarget);
  const setDefaultTarget = useLocationActionsStore((state) => state.setDefaultTarget);

  // The browser cannot launch native apps, so the whole entry point stays hidden there.
  if (locationActions.kind !== "supported") return null;

  const hasTarget = Boolean(taskId) || Boolean(projectPath);
  const DefaultIcon = OPENER_ICONS[defaultTarget];

  /**
   * Resolves the absolute path to act on. A task resolves its worktree directory live
   * through the backend (the only source of truth for where the session runs); otherwise
   * the known project root is used. Returns null when nothing can be resolved.
   */
  const resolvePath = async (): Promise<string | null> => {
    if (taskId) {
      try {
        return await locationActions.resolveTaskCwd(taskId);
      } catch {
        return null;
      }
    }
    return projectPath ?? null;
  };

  const openWith = async (target: LocationTarget) => {
    const path = await resolvePath();
    if (path === null) {
      toast.error(t("locationActions.pathUnavailable"));
      return;
    }
    try {
      await locationActions.open(target, path);
    } catch {
      toast.error(t("locationActions.openFailed", { app: t(OPENER_LABEL_KEYS[target as DefaultLocationTarget]) }));
    }
  };

  const copyPath = async () => {
    const path = await resolvePath();
    if (path === null) {
      toast.error(t("locationActions.pathUnavailable"));
      return;
    }
    try {
      await navigator.clipboard.writeText(path);
      toast.success(t("locationActions.copied"));
    } catch {
      toast.error(t("locationActions.copyFailed"));
    }
  };

  const chooseOpener = (target: DefaultLocationTarget) => {
    setDefaultTarget(target);
    void openWith(target);
  };

  return (
    <div
      className="inline-flex items-center overflow-hidden rounded-md border border-border/60"
      role="group"
      aria-label={t("locationActions.group")}
    >
      <Tooltip>
        <TooltipTrigger
          render={
            <button
              type="button"
              disabled={!hasTarget}
              aria-label={t("locationActions.openWith", { app: t(OPENER_LABEL_KEYS[defaultTarget]) })}
              onClick={() => void openWith(defaultTarget)}
              className={cn(
                "flex size-8 items-center justify-center rounded-l-md text-muted-foreground outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring",
                "hover:bg-muted hover:text-foreground",
                "disabled:pointer-events-none disabled:opacity-40",
              )}
            />
          }
        >
          <DefaultIcon className="size-4" />
        </TooltipTrigger>
        <TooltipContent>
          {hasTarget
            ? t("locationActions.openWith", { app: t(OPENER_LABEL_KEYS[defaultTarget]) })
            : t("locationActions.pathUnavailable")}
        </TooltipContent>
      </Tooltip>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              type="button"
              variant="ghost"
              size="icon"
              disabled={!hasTarget}
              aria-label={t("locationActions.menu")}
              className="size-8 rounded-l-none rounded-r-md border-l border-border/60 text-muted-foreground hover:text-foreground disabled:pointer-events-none disabled:opacity-40"
            />
          }
        >
          <IconChevronDown className="size-3 opacity-60" aria-hidden="true" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-44">
          <DropdownMenuItem
            className="gap-2 rounded-sm px-2 py-1.5 text-xs"
            onClick={() => void copyPath()}
          >
            <IconCopy className="size-3.5 shrink-0" />
            {t("locationActions.copyPath")}
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          {OPENERS.map((target) => {
            const Icon = OPENER_ICONS[target];
            return (
              <DropdownMenuItem
                key={target}
                className="gap-2 rounded-sm px-2 py-1.5 text-xs"
                onClick={() => chooseOpener(target)}
              >
                <Icon className="size-3.5 shrink-0" />
                {t(OPENER_LABEL_KEYS[target])}
                {target === defaultTarget && <IconCheck className="ml-auto size-4" />}
              </DropdownMenuItem>
            );
          })}
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
