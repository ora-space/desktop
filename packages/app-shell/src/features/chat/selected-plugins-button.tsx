import { useTranslation } from "react-i18next";
import { IconPlug, IconX } from "@tabler/icons-react";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@ora/ui";
import type { PluginEntry } from "../settings/plugin-catalog";

/** Icons beyond this count collapse into a "+N" badge so the stack stays compact. */
const STACK_ICON_LIMIT = 3;

/**
 * Shows the plugins applied to the current message as an overlapping icon stack.
 * Clicking it lists only those already-applied plugins (picked via "@" or the "+"
 * menu) so they can be reviewed and removed without reopening either menu.
 */
export function SelectedPluginsButton({
  selected,
  onRemove,
  disabled = false,
}: {
  selected: PluginEntry[];
  onRemove: (plugin: PluginEntry) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const stacked = selected.slice(0, STACK_ICON_LIMIT);
  const overflow = selected.length - stacked.length;

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={disabled}
            aria-label={t("chat.plugins.applied", { count: selected.length })}
            className="h-7 gap-1 rounded-md px-1.5 text-muted-foreground hover:bg-muted/60 hover:text-foreground"
          />
        }
      >
        <span className="flex items-center -space-x-2.5">
          {stacked.map((plugin) => (
            <span
              key={plugin.id}
              className="flex size-5 shrink-0 items-center justify-center rounded-full border border-border bg-background text-muted-foreground"
            >
              <IconPlug className="size-3" />
            </span>
          ))}
        </span>
        {overflow > 0 && (
          <span className="whitespace-nowrap text-xs">+{overflow}</span>
        )}
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" side="top" className="w-56">
        {selected.map((plugin) => (
          <DropdownMenuItem
            key={plugin.id}
            onClick={() => onRemove(plugin)}
            className="gap-1.5 rounded-sm px-2 py-1.5 text-xs"
          >
            <IconPlug className="size-3.5 shrink-0 text-muted-foreground" />
            <span className="min-w-0 flex-1 truncate">{plugin.name}</span>
            <IconX
              className="size-3.5 shrink-0 text-muted-foreground"
              aria-hidden="true"
            />
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
