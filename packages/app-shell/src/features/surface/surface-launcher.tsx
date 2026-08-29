import {
  Button,
  cn,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@ora/ui";
import { IconWorld } from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";
import { useSurfaceStore } from "../../state/stores/surface-store";
import { PluginLogoMark } from "../settings/plugin-logo";
import { listSurfaceDefinitions } from "./surface-definitions";
import { useOpenSurface } from "./use-open-surface";

/**
 * The header entry for plugin surfaces: hidden without any, a direct button for
 * one, and a menu when several plugins contribute surfaces. Menu rows lead with
 * the plugin's logo, truncate long titles (the row's `title` keeps the full
 * name reachable), and mark live embedded instances with the slot owner
 * highlighted, because the launcher is the only switcher between them. It must
 * stay a sibling of `DragRegion`, whose children are pointer-inert.
 */
export function SurfaceLauncher() {
  const { t } = useTranslation();
  const definitions = listSurfaceDefinitions(useInstalledPlugins().data ?? []);
  const openSurface = useOpenSurface();
  const records = useSurfaceStore((state) => state.records);
  const sidePanelInstance = useSurfaceStore((state) => state.sidePanelInstance);
  if (definitions.length === 0) return null;
  if (definitions.length === 1) {
    const [definition] = definitions;
    return (
      <Button
        variant="ghost"
        size="icon"
        aria-label={definition.title}
        title={definition.title}
        onClick={() => void openSurface(definition)}
      >
        <IconWorld />
      </Button>
    );
  }
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            aria-label={t("surface.launcher")}
            title={t("surface.launcher")}
          />
        }
      >
        <IconWorld />
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="end"
        className="max-w-[min(320px,calc(100vw-2rem))]"
      >
        {definitions.map((definition) => {
          const shownInstance = Object.values(records).find(
            (record) =>
              record.pluginId === definition.pluginId &&
              record.target === "embedded",
          );
          return (
            <DropdownMenuItem
              key={definition.pluginId}
              onClick={() => void openSurface(definition)}
              title={definition.title}
              className="min-w-0"
            >
              <PluginLogoMark
                logo={definition.logo}
                fallback={IconWorld}
                className="size-4 shrink-0 text-muted-foreground"
              />
              <span className="min-w-0 flex-1 truncate">
                {definition.title}
              </span>
              <span className="ml-2 min-w-0 shrink truncate text-xs text-muted-foreground">
                {definition.pluginDisplayName}
              </span>
              {shownInstance !== undefined && (
                <span
                  aria-hidden="true"
                  className={cn(
                    "ml-1.5 size-1.5 shrink-0 rounded-full",
                    sidePanelInstance === shownInstance.instance
                      ? "bg-foreground"
                      : "bg-muted-foreground/40",
                  )}
                />
              )}
            </DropdownMenuItem>
          );
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
