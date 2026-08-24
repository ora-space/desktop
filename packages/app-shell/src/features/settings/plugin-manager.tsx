import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { InstalledPlugin } from "@ora/contracts";
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
  DropdownMenuTrigger,
  Input,
  Switch,
} from "@ora/ui";
import {
  IconDots,
  IconLoader2,
  IconRefresh,
  IconSearch,
  IconTrash,
} from "@tabler/icons-react";
import { filterDiscoveredPlugins } from "./filter-discovered-plugins";
import { PluginLogo } from "./plugin-logo";
import { usePluginMutations } from "../../state/hooks/use-plugin-mutations";
import { usePluginScan } from "../../state/hooks/use-plugin-scan";

/** The installed-plugin manager drives durable state through the backend lifecycle commands. */
export function PluginManager({
  plugins,
  onBack,
}: {
  plugins: InstalledPlugin[];
  onBack: () => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const scan = usePluginScan();

  const needle = query.trim().toLowerCase();
  const visible = useMemo(
    () => filterDiscoveredPlugins(plugins, needle),
    [needle, plugins],
  );

  return (
    <div className="space-y-5">
      <Breadcrumb>
        <BreadcrumbList>
          <BreadcrumbItem>
            <BreadcrumbLink render={<button type="button" onClick={onBack} />}>
              {t("settings.plugins.title")}
            </BreadcrumbLink>
          </BreadcrumbItem>
          <BreadcrumbSeparator />
          <BreadcrumbItem>
            <BreadcrumbPage>
              {t("settings.plugins.manageInstalled")}
            </BreadcrumbPage>
          </BreadcrumbItem>
        </BreadcrumbList>
      </Breadcrumb>

      <header>
        <h2 className="text-lg font-semibold">{t("settings.plugins.title")}</h2>
        <p className="mt-1 max-w-2xl text-sm leading-6 text-muted-foreground">
          {t("settings.plugins.manageDescription")}
        </p>
      </header>

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <Badge
          variant="secondary"
          className="h-7 shrink-0 gap-1.5 rounded-lg px-2.5 text-sm font-medium"
        >
          {t("settings.plugins.installed")}
          <span className="font-normal text-muted-foreground">
            {plugins.length}
          </span>
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
        <Button
          variant="outline"
          size="sm"
          disabled={scan.isPending}
          onClick={() => scan.mutate()}
          aria-label={t("settings.plugins.scanInstalled")}
        >
          {scan.isPending ? (
            <IconLoader2 className="animate-spin" />
          ) : (
            <IconRefresh />
          )}
        </Button>
      </div>

      {visible.length === 0 ? (
        <p className="py-10 text-center text-sm text-muted-foreground">
          {plugins.length === 0
            ? t("settings.plugins.noneInstalled")
            : t("settings.plugins.empty")}
        </p>
      ) : (
        <div className="divide-y divide-border border-y border-border">
          {visible.map((plugin) => (
            <InstalledPluginRow key={plugin.id} plugin={plugin} />
          ))}
        </div>
      )}
    </div>
  );
}

function InstalledPluginRow({ plugin }: { plugin: InstalledPlugin }) {
  const { t } = useTranslation();
  const mutations = usePluginMutations(
    plugin.id,
    plugin.kind === "agent" ? plugin.name : undefined,
  );
  const enabling = mutations.enable.isPending;
  const disabling = mutations.disable.isPending;
  const uninstalling = mutations.uninstall.isPending;
  const busy = enabling || disabling || uninstalling;

  return (
    <div className="flex items-center gap-3 py-3">
      <PluginLogo logo={plugin.logo} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">
          {plugin.displayName}
        </span>
        <span className="block truncate text-xs text-muted-foreground">
          {plugin.id}
        </span>
        <span className="mt-0.5 block truncate text-[11px] text-muted-foreground/80">
          {plugin.version} · {plugin.kind} ·{" "}
          {plugin.runtime === "failed" ? plugin.failureReason : plugin.runtime}
        </span>
      </span>

      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("settings.plugins.openMenu", {
                name: plugin.displayName,
              })}
              className="shrink-0 text-muted-foreground"
              disabled={busy}
            />
          }
        >
          <IconDots />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-44">
          <DropdownMenuItem
            variant="destructive"
            disabled={uninstalling}
            onClick={() => mutations.uninstall.mutate()}
          >
            {uninstalling ? (
              <IconLoader2 className="animate-spin" />
            ) : (
              <IconTrash />
            )}
            {t(
              uninstalling
                ? "settings.plugins.uninstalling"
                : "settings.plugins.uninstall",
            )}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <Switch
        checked={plugin.enabled}
        disabled={busy}
        onCheckedChange={(next) => {
          if (next) void mutations.enable.mutate();
          else void mutations.disable.mutate();
        }}
        aria-label={t("settings.plugins.toggleSkill", {
          name: plugin.displayName,
        })}
      />
      {(enabling || disabling) && (
        <IconLoader2 className="size-4 shrink-0 animate-spin text-muted-foreground" />
      )}
    </div>
  );
}
