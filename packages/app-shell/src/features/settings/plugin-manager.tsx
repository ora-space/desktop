import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AvailablePlugin, InstalledPlugin } from "@ora/contracts";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
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
  toast,
} from "@ora/ui";
import {
  IconArrowBigUpLines,
  IconDots,
  IconLoader2,
  IconProgressDown,
  IconRefresh,
  IconSearch,
  IconSettingsBolt,
  IconTrash,
} from "@tabler/icons-react";
import { filterDiscoveredPlugins } from "./filter-discovered-plugins";
import { localizeContractError } from "../../i18n/contract-error";
import { PluginLogo } from "./plugin-logo";
import { usePluginMutations } from "../../state/hooks/use-plugin-mutations";
import { usePluginScan } from "../../state/hooks/use-plugin-scan";
import { useUpdatePlugin } from "../../state/hooks/use-update-plugin";

/** The installed-plugin manager exposes runtime and package lifecycle commands. */
export function PluginManager({
  plugins,
  onBack,
  onConfigure,
  availableById,
}: {
  plugins: InstalledPlugin[];
  onBack: () => void;
  onConfigure: (plugin: Pick<InstalledPlugin, "id" | "displayName">) => void;
  availableById?: ReadonlyMap<string, AvailablePlugin>;
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
            <InstalledPluginRow
              key={plugin.id}
              plugin={plugin}
              onConfigure={onConfigure}
              available={availableById?.get(plugin.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function InstalledPluginRow({
  plugin,
  onConfigure,
  available,
}: {
  plugin: InstalledPlugin;
  onConfigure: (plugin: Pick<InstalledPlugin, "id" | "displayName">) => void;
  available: AvailablePlugin | undefined;
}) {
  const { t } = useTranslation();
  const update = useUpdatePlugin(plugin.id);
  const mutations = usePluginMutations(
    plugin.id,
    plugin.kind === "agent" ? plugin.name : undefined,
  );
  const uninstalling = mutations.uninstall.isPending;
  const busy = uninstalling || update.isPending;
  const hasUpdate =
    available !== undefined && available.version !== plugin.version;
  const [uninstallOpen, setUninstallOpen] = useState(false);
  const [deleteData, setDeleteData] = useState(true);
  const failUpdate = (cause: unknown) => {
    toast.error(t("settings.plugins.updateFailed"), {
      description: localizeContractError(cause, t),
    });
  };
  const failUninstall = (cause: unknown) => {
    toast.error(t("settings.plugins.uninstallFailed"), {
      description: localizeContractError(cause, t),
    });
  };

  return (
    <>
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
            {plugin.runtime === "failed"
              ? plugin.failureReason
              : plugin.runtime}
            {plugin.kind === "hook" &&
              ` · ${plugin.protocol} · ${plugin.command}${plugin.target ? ` · ${plugin.target}` : ""} · ${plugin.toolVersion}`}
          </span>
          {plugin.configuration.state === "available" &&
            plugin.configuration.completeness === "incomplete" && (
              <Badge variant="secondary" className="mt-1">
                {t("settings.plugins.configuration.needsConfiguration")}
              </Badge>
            )}
          {plugin.configuration.state === "unavailable" && (
            <Badge variant="destructive" className="mt-1">
              {t("settings.plugins.configuration.unavailableBadge")}
            </Badge>
          )}
          {plugin.installationValidity.validity === "invalid_declaration" && (
            <Badge variant="destructive" className="mt-1">
              {t("settings.plugins.invalidDeclaration")}
            </Badge>
          )}
        </span>

        {hasUpdate && (
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => update.mutate({}, { onError: failUpdate })}
          >
            {update.isPending ? <IconProgressDown /> : <IconArrowBigUpLines />}
            {t("settings.plugins.update")}
          </Button>
        )}

        {plugin.installationValidity.validity === "valid" &&
          plugin.configuration.state !== "not_declared" && (
            <Button
              variant="outline"
              size="sm"
              disabled={busy}
              onClick={() => onConfigure(plugin)}
            >
              <IconSettingsBolt />
              {t("settings.plugins.configuration.configure")}
            </Button>
          )}

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
              onClick={() => setUninstallOpen(true)}
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
      </div>
      <AlertDialog
        open={uninstallOpen}
        onOpenChange={(open) => {
          setUninstallOpen(open);
          if (open) setDeleteData(true);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.plugins.uninstallTitle", {
                name: plugin.displayName,
              })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.plugins.uninstallDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={deleteData}
              onChange={(event) => setDeleteData(event.target.checked)}
            />
            {t("settings.plugins.deleteConfigurationData")}
          </label>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={uninstalling}>
              {t("common.cancel")}
            </AlertDialogCancel>
            <Button
              variant="destructive"
              disabled={uninstalling}
              onClick={() =>
                mutations.uninstall.mutate(deleteData ? "delete" : "retain", {
                  onError: failUninstall,
                  onSuccess: () => setUninstallOpen(false),
                })
              }
            >
              {t("settings.plugins.uninstall")}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
