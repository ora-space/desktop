import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@ora/ui";
import { IconWorld } from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";
import { listSurfaceDefinitions } from "./surface-definitions";
import { useOpenSurface } from "./use-open-surface";

/**
 * The header entry for plugin surfaces: hidden without any, a direct button for
 * one, and a menu when several plugins contribute surfaces.
 *
 * It must stay a sibling of `DragRegion`, whose children are pointer-inert.
 */
export function SurfaceLauncher() {
  const { t } = useTranslation();
  const definitions = listSurfaceDefinitions(useInstalledPlugins().data ?? []);
  const openSurface = useOpenSurface();
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
      <DropdownMenuContent align="end">
        {definitions.map((definition) => (
          <DropdownMenuItem
            key={definition.pluginId}
            onClick={() => void openSurface(definition)}
          >
            {definition.title}
            <span className="ml-2 text-xs text-muted-foreground">
              {definition.pluginDisplayName}
            </span>
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
