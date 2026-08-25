import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AvailablePlugin, InstalledPlugin } from "@ora/contracts";
import { Button, Input, toast } from "@ora/ui";
import {
  IconLoader2,
  IconRefresh,
  IconSearch,
  IconUpload,
} from "@tabler/icons-react";
import { localizeContractError } from "../../i18n/contract-error";
import { usePlatform } from "../../platform";
import { useAvailablePlugins } from "../../state/hooks/use-available-plugins";
import { useInstallPlugin } from "../../state/hooks/use-install-plugin";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";
import { usePluginImport } from "../../state/hooks/use-plugin-import";
import { usePluginMutations } from "../../state/hooks/use-plugin-mutations";
import { usePluginRegistrySync } from "../../state/hooks/use-plugin-registry-sync";
import { PluginLogo } from "./plugin-logo";
import { PluginManager } from "./plugin-manager";

/**
 * The plugin marketplace pane backed by the registry contract: the browse grid reads the
 * cached registry index, installs and lifecycle changes go through the backend commands,
 * and the installed-plugin manager drives the durable lifecycle surface.
 */
export function PluginsSettings() {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [managing, setManaging] = useState(false);
  const [selecting, setSelecting] = useState(false);

  const platform = usePlatform();
  const available = useAvailablePlugins();
  const installed = useInstalledPlugins();
  const sync = usePluginRegistrySync();
  const importPlugin = usePluginImport();

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
          [
            plugin.title,
            plugin.name,
            plugin.kind,
            plugin.namespace,
            plugin.description,
            plugin.id,
          ].some((value) => value.toLowerCase().includes(needle)),
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

  const handleImport = async () => {
    setSelecting(true);
    try {
      const path = await platform.selectPath({ kind: "file" });
      if (path === null) return;
      importPlugin.mutate(
        { path },
        {
          onSuccess: () => toast.success(t("settings.plugins.importSuccess")),
          onError: (cause) =>
            toast.error(t("settings.plugins.importFailed"), {
              description: localizeContractError(cause, t),
            }),
        },
      );
    } catch (error) {
      // Surface the picker failure through the toast instead of the console: app-shell tests
      // run under a clean-stderr gate, so a console write here would fail the whole suite.
      toast.error(t("settings.plugins.pathSelectionError"), {
        description: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setSelecting(false);
    }
  };

  if (managing) {
    return (
      <PluginManager
        plugins={installed.data ?? []}
        onBack={() => setManaging(false)}
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
          onClick={() =>
            sync.mutate(undefined, {
              onError: (cause) => {
                toast.error(t("settings.plugins.syncFailed"), {
                  description: localizeContractError(cause, t),
                });
              },
            })
          }
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
        <Button
          variant="outline"
          size="sm"
          disabled={importPlugin.isPending || selecting}
          onClick={() => void handleImport()}
          aria-label={t("settings.plugins.import")}
        >
          {importPlugin.isPending || selecting ? (
            <IconLoader2 className="animate-spin" />
          ) : (
            <IconUpload />
          )}
          <span className="hidden sm:inline">
            {t("settings.plugins.import")}
          </span>
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
  const mutations = usePluginMutations(
    plugin.id,
    installed?.kind === "agent" ? installed.name : undefined,
  );
  const busy = install.isPending || mutations.uninstall.isPending;

  const failInstall = (cause: unknown) => {
    toast.error(t("settings.plugins.installFailed"), {
      description: localizeContractError(cause, t),
    });
  };

  return (
    <div className="flex items-center gap-3 py-3">
      <PluginLogo logo={plugin.logo} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">
          {plugin.title || plugin.name}
        </span>
        <span className="block truncate text-xs text-muted-foreground">
          {[plugin.name, plugin.namespace, plugin.kind, plugin.version]
            .filter(Boolean)
            .join(" · ")}
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
          onClick={() =>
            mutations.uninstall.mutate(undefined, { onError: failInstall })
          }
        >
          {t("settings.plugins.uninstall")}
        </Button>
      )}
    </div>
  );
}
