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
  Button,
  Input,
  toast,
} from "@ora/ui";
import { IconLoader2, IconRefresh, IconSearch } from "@tabler/icons-react";
import { localizeContractError } from "../../i18n/contract-error";
import { useAvailablePlugins } from "../../state/hooks/use-available-plugins";
import { useInstallPlugin } from "../../state/hooks/use-install-plugin";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";
import { usePluginMutations } from "../../state/hooks/use-plugin-mutations";
import { usePluginRegistrySync } from "../../state/hooks/use-plugin-registry-sync";
import { PluginLogo } from "./plugin-logo";
import { PluginManager } from "./plugin-manager";
import { PluginConfigurationEditor } from "./plugin-configuration-editor";

/**
 * The plugin marketplace pane backed by the registry contract: the browse grid reads the
 * cached registry index, installs and lifecycle changes go through the backend commands,
 * and the installed-plugin manager drives the durable lifecycle surface.
 */
export function PluginsSettings() {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [managing, setManaging] = useState(false);
  const [configurationPluginId, setConfigurationPluginId] = useState<
    string | null
  >(null);

  const available = useAvailablePlugins();
  const installed = useInstalledPlugins();
  const sync = usePluginRegistrySync();

  const installedById = useMemo(() => {
    const byId = new Map<string, InstalledPlugin>();
    for (const plugin of installed.data ?? []) byId.set(plugin.id, plugin);
    return byId;
  }, [installed.data]);

  const needle = query.trim().toLowerCase();
  const visiblePlugins = useMemo(
    () =>
      (available.data?.plugins ?? []).filter(
        (plugin) =>
          !needle ||
          [plugin.name, plugin.namespace, plugin.description, plugin.id].some(
            (value) => value.toLowerCase().includes(needle),
          ),
      ),
    [available.data, needle],
  );

  const updatedAt = available.data?.updatedAt;
  const lastSynced =
    updatedAt === undefined || updatedAt === 0n
      ? t("settings.plugins.neverSynced")
      : t("settings.plugins.lastSynced", {
          time: new Date(Number(updatedAt) * 1000).toLocaleString(),
        });

  if (managing) {
    const configurationPlugin = installed.data?.find(
      (plugin) => plugin.id === configurationPluginId,
    );
    if (configurationPlugin !== undefined) {
      return (
        <PluginConfigurationEditor
          pluginId={configurationPlugin.id}
          displayName={configurationPlugin.displayName}
          onBack={() => setConfigurationPluginId(null)}
        />
      );
    }
    return (
      <PluginManager
        plugins={installed.data ?? []}
        onBack={() => setManaging(false)}
        onConfigure={setConfigurationPluginId}
      />
    );
  }

  return (
    <div className="space-y-5">
      <header>
        <h2 className="text-lg font-semibold">{t("settings.plugins.title")}</h2>
        <p className="mt-1 max-w-2xl text-sm leading-6 text-muted-foreground">
          {t("settings.plugins.description")}
        </p>
      </header>

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <Button
          variant="outline"
          size="sm"
          disabled={sync.isPending}
          onClick={() => sync.mutate()}
          aria-label={t("settings.plugins.syncMarketplace")}
        >
          {sync.isPending ? (
            <IconLoader2 className="animate-spin" />
          ) : (
            <IconRefresh />
          )}
          <span className="hidden sm:inline">
            {t("settings.plugins.syncMarketplace")}
          </span>
        </Button>
        <span className="text-xs text-muted-foreground">{lastSynced}</span>
        <div className="relative min-w-0 flex-1 sm:ml-auto">
          <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t("settings.plugins.search")}
            aria-label={t("settings.plugins.search")}
            className="pl-8"
          />
        </div>
        <Button variant="outline" size="sm" onClick={() => setManaging(true)}>
          {t("settings.plugins.manageInstalled")}
        </Button>
      </div>

      {visiblePlugins.length === 0 ? (
        <p className="py-10 text-center text-sm text-muted-foreground">
          {t("settings.plugins.empty")}
        </p>
      ) : (
        <div className="divide-y divide-border border-y border-border">
          {visiblePlugins.map((plugin) => (
            <AvailablePluginRow
              key={plugin.id}
              plugin={plugin}
              installed={installedById.get(plugin.id)}
            />
          ))}
        </div>
      )}
    </div>
  );
}

/** One registry entry with backend-driven install and uninstall actions. */
function AvailablePluginRow({
  plugin,
  installed,
}: {
  plugin: AvailablePlugin;
  installed: InstalledPlugin | undefined;
}) {
  const { t } = useTranslation();
  const install = useInstallPlugin(plugin.id);
  const mutations = usePluginMutations(plugin.id);
  const busy = install.isPending || mutations.uninstall.isPending;
  const [uninstallOpen, setUninstallOpen] = useState(false);
  const [deleteData, setDeleteData] = useState(true);

  const failInstall = (cause: unknown) => {
    toast.error(t("settings.plugins.installFailed"), {
      description: localizeContractError(cause, t),
    });
  };

  return (
    <>
      <div className="flex items-center gap-3 py-3">
        <PluginLogo logo={plugin.logo} />
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-medium">
            {plugin.name}
          </span>
          <span className="block truncate text-xs text-muted-foreground">
            {plugin.namespace} · {plugin.version}
          </span>
          {plugin.description !== "" && (
            <span className="mt-0.5 block truncate text-[11px] text-muted-foreground/80">
              {plugin.description}
            </span>
          )}
        </span>
        {busy ? (
          <Button variant="outline" size="sm" disabled className="shrink-0">
            <IconLoader2 className="animate-spin" />
            {t(
              installed === undefined
                ? "settings.plugins.installing"
                : "settings.plugins.uninstalling",
            )}
          </Button>
        ) : installed === undefined ? (
          <Button
            variant="outline"
            size="sm"
            className="shrink-0"
            onClick={() => install.mutate({}, { onError: failInstall })}
          >
            {t("settings.plugins.install")}
          </Button>
        ) : (
          <Button
            variant="outline"
            size="sm"
            className="shrink-0"
            onClick={() => setUninstallOpen(true)}
          >
            {t("settings.plugins.uninstall")}
          </Button>
        )}
      </div>
      <AlertDialog open={uninstallOpen} onOpenChange={setUninstallOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.plugins.uninstallTitle", { name: plugin.name })}
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
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <Button
              variant="destructive"
              onClick={() =>
                mutations.uninstall.mutate(deleteData ? "delete" : "retain", {
                  onError: failInstall,
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
