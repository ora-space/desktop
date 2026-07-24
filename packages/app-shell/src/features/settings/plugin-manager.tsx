import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Badge,
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Input,
  Switch,
} from "@ora/ui";
import { IconDots, IconInfoCircle, IconSearch, IconTrash } from "@tabler/icons-react";
import type { PluginEntry } from "./plugin-catalog";
import { PluginTile } from "./plugin-tile";

/**
 * The installed-plugin manager reached from the gear beside "Installed". It lists
 * plugins only — skills, MCP servers and apps are deliberately absent — with one
 * switch per row so a plugin can be turned off without uninstalling it.
 */
export function PluginManager({ plugins, disabledIds, onBack, onOpen, onToggleEnabled, onUninstall }: {
  plugins: PluginEntry[];
  disabledIds: string[];
  onBack: () => void;
  onOpen: (id: string) => void;
  onToggleEnabled: (id: string) => void;
  onUninstall: (id: string) => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");

  const needle = query.trim().toLowerCase();
  const visible = useMemo(() => plugins.filter((plugin) => !needle
    || plugin.name.toLowerCase().includes(needle)
    || plugin.publisher.toLowerCase().includes(needle)
    || t(plugin.summaryKey).toLowerCase().includes(needle)), [needle, plugins, t]);

  return (
    <div className="space-y-5">
      <Breadcrumb>
        <BreadcrumbList>
          <BreadcrumbItem>
            <BreadcrumbLink render={<button type="button" onClick={onBack} />}>{t("settings.plugins.title")}</BreadcrumbLink>
          </BreadcrumbItem>
          <BreadcrumbSeparator />
          <BreadcrumbItem><BreadcrumbPage>{t("settings.plugins.manageInstalled")}</BreadcrumbPage></BreadcrumbItem>
        </BreadcrumbList>
      </Breadcrumb>

      <header>
        <h2 className="text-lg font-semibold">{t("settings.plugins.title")}</h2>
        <p className="mt-1 max-w-2xl text-sm leading-6 text-muted-foreground">{t("settings.plugins.manageDescription")}</p>
      </header>

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <Badge variant="secondary" className="h-7 shrink-0 gap-1.5 rounded-lg px-2.5 text-sm font-medium">
          {t("settings.plugins.title")}
          <span className="font-normal text-muted-foreground">{plugins.length}</span>
        </Badge>
        <div className="relative min-w-0 flex-1 sm:max-w-xs sm:ml-auto">
          <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("settings.plugins.search")}
            aria-label={t("settings.plugins.search")}
            className="pl-8"
          />
        </div>
      </div>

      {visible.length === 0
        ? <p className="py-10 text-center text-sm text-muted-foreground">{plugins.length === 0 ? t("settings.plugins.noneInstalled") : t("settings.plugins.empty")}</p>
        : (
          <div className="divide-y divide-border border-y border-border">
            {visible.map((plugin) => (
              <div key={plugin.id} className="flex items-center gap-3 py-3">
                <button
                  type="button"
                  onClick={() => onOpen(plugin.id)}
                  className="flex min-w-0 flex-1 items-center gap-3 rounded-md text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <PluginTile plugin={plugin} />
                  <span className="min-w-0">
                    <span className="block truncate text-sm font-medium">{plugin.name}</span>
                    <span className="block truncate text-xs text-muted-foreground">{t(plugin.summaryKey)}</span>
                    <span className="mt-0.5 block truncate text-[11px] text-muted-foreground/80">{plugin.publisher}</span>
                  </span>
                </button>
                <DropdownMenu>
                  <DropdownMenuTrigger
                    render={<Button variant="ghost" size="icon-sm" aria-label={t("settings.plugins.openMenu", { name: plugin.name })} className="shrink-0 text-muted-foreground" />}
                  >
                    <IconDots />
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end" className="w-44">
                    <DropdownMenuItem onClick={() => onOpen(plugin.id)}><IconInfoCircle />{t("settings.plugins.viewDetails")}</DropdownMenuItem>
                    <DropdownMenuSeparator />
                    <DropdownMenuItem variant="destructive" onClick={() => onUninstall(plugin.id)}><IconTrash />{t("settings.plugins.uninstall")}</DropdownMenuItem>
                  </DropdownMenuContent>
                </DropdownMenu>
                <Switch
                  checked={!disabledIds.includes(plugin.id)}
                  onCheckedChange={() => onToggleEnabled(plugin.id)}
                  aria-label={t("settings.plugins.toggleSkill", { name: plugin.name })}
                />
              </div>
            ))}
          </div>
        )}
    </div>
  );
}
