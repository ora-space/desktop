import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AvailablePlugin, InstalledPlugin } from "@ora/contracts";
import { Button, Input, toast } from "@ora/ui";
import {
  IconArrowBigUpLines,
  IconCircleCheck,
  IconLoader2,
  IconPlus,
  IconProgressDown,
  IconRefresh,
  IconSearch,
  IconSettings,
  IconUpload,
} from "@tabler/icons-react";
import { localizeContractError } from "../../i18n/contract-error";
import { usePlatform } from "../../platform";
import { useAvailablePlugins } from "../../state/hooks/use-available-plugins";
import { useInstallPlugin } from "../../state/hooks/use-install-plugin";
import { useUpdatePlugin } from "../../state/hooks/use-update-plugin";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";
import { usePluginImport } from "../../state/hooks/use-plugin-import";
import { usePluginRegistrySync } from "../../state/hooks/use-plugin-registry-sync";
import { PluginLogo } from "./plugin-logo";
import { PluginSourcesManager } from "./plugin-sources-manager";
import { PluginManager } from "./plugin-manager";
import { PluginConfigurationEditor } from "./plugin-configuration-editor";
import type { PluginConfigurationNavigationGuard } from "./plugin-configuration-editor";

/** The registry kind order shown in the marketplace, mirroring the contracts docs. */
const MARKETPLACE_KIND_ORDER = [
  "agent",
  "workbench",
  "webview",
  "skill",
  "mcp",
];

/** Readable marketplace section labels for the known plugin kinds. */
const MARKETPLACE_KIND_LABELS: Record<string, string> = {
  agent: "Agent",
  workbench: "Workbench",
  webview: "Webview",
  skill: "Skill",
  mcp: "MCP",
};

/**
 * The plugin marketplace pane backed by the registry contract: the browse grid reads the
 * cached registry index, installs and lifecycle changes go through the backend commands,
 * and the installed-plugin manager drives the durable lifecycle surface.
 */
export function PluginsSettings({
  onNavigationGuardChange,
}: {
  onNavigationGuardChange?: (
    guard: PluginConfigurationNavigationGuard | null,
  ) => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [managing, setManaging] = useState(false);
  const [managingSources, setManagingSources] = useState(false);
  const [configurationPlugin, setConfigurationPlugin] = useState<{
    id: string;
    displayName: string;
  } | null>(null);
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

  const availableById = useMemo(() => {
    const byId = new Map<string, AvailablePlugin>();
    for (const plugin of available.data?.plugins ?? [])
      byId.set(plugin.id, plugin);
    return byId;
  }, [available.data]);

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

  const groupedPlugins = useMemo(() => {
    const byKind = new Map<string, AvailablePlugin[]>();
    for (const plugin of visiblePlugins) {
      const group = byKind.get(plugin.kind) ?? [];
      group.push(plugin);
      byKind.set(plugin.kind, group);
    }
    return [...byKind.entries()].sort(([left], [right]) => {
      const leftRank = MARKETPLACE_KIND_ORDER.indexOf(left);
      const rightRank = MARKETPLACE_KIND_ORDER.indexOf(right);
      const leftIndex =
        leftRank === -1 ? MARKETPLACE_KIND_ORDER.length : leftRank;
      const rightIndex =
        rightRank === -1 ? MARKETPLACE_KIND_ORDER.length : rightRank;
      return leftIndex - rightIndex || left.localeCompare(right);
    });
  }, [visiblePlugins]);

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

  if (managingSources) {
    return <PluginSourcesManager onBack={() => setManagingSources(false)} />;
  }

  if (managing) {
    if (configurationPlugin !== null) {
      return (
        <PluginConfigurationEditor
          pluginId={configurationPlugin.id}
          displayName={configurationPlugin.displayName}
          onBack={() => setConfigurationPlugin(null)}
          onNavigationGuardChange={onNavigationGuardChange}
        />
      );
    }
    return (
      <PluginManager
        plugins={installed.data ?? []}
        onBack={() => setManaging(false)}
        availableById={availableById}
        onConfigure={(plugin) =>
          setConfigurationPlugin({
            id: plugin.id,
            displayName: plugin.displayName,
          })
        }
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

      <div className="space-y-3">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
          <div className="relative min-w-0 flex-1">
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
            className="shrink-0"
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
        </div>

        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <span className="text-xs text-muted-foreground">{lastSynced}</span>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setManaging(true)}
            >
              <IconSettings />
              {t("settings.plugins.manageInstalled")}
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setManagingSources(true)}
            >
              <IconSettings />
              {t("settings.plugins.manageSources")}
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
        </div>
      </div>

      {visiblePlugins.length === 0 ? (
        <p className="py-10 text-center text-sm text-muted-foreground">
          {t("settings.plugins.empty")}
        </p>
      ) : (
        <div className="space-y-6">
          {groupedPlugins.map(([kind, plugins]) => (
            <section key={kind}>
              <h3 className="mb-2 text-sm font-semibold">
                {MARKETPLACE_KIND_LABELS[kind] ?? kind}
              </h3>
              <div className="grid gap-3 sm:grid-cols-2">
                {plugins.map((plugin) => (
                  <AvailablePluginCard
                    key={plugin.id}
                    plugin={plugin}
                    installed={installedById.get(plugin.id)}
                  />
                ))}
              </div>
            </section>
          ))}
        </div>
      )}
    </div>
  );
}

/** One marketplace entry presented as a compact card with its brand, title, and summary. */
function AvailablePluginCard({
  plugin,
  installed,
}: {
  plugin: AvailablePlugin;
  installed: InstalledPlugin | undefined;
}) {
  const { t } = useTranslation();
  const install = useInstallPlugin(plugin.id);
  const update = useUpdatePlugin(plugin.id);
  const busy = install.isPending || update.isPending;
  const hasUpdate = plugin.version !== installed?.version;

  const failInstall = (cause: unknown) => {
    toast.error(t("settings.plugins.installFailed"), {
      description: localizeContractError(cause, t),
    });
  };
  const failUpdate = (cause: unknown) => {
    toast.error(t("settings.plugins.updateFailed"), {
      description: localizeContractError(cause, t),
    });
  };

  return (
    <div className="flex items-start gap-3 rounded-lg border border-border p-3">
      <PluginLogo logo={plugin.logo} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">
          {plugin.title || plugin.name}
        </span>
        {plugin.description !== "" && (
          <span className="mt-0.5 block line-clamp-2 text-xs leading-5 text-muted-foreground">
            {plugin.description}
          </span>
        )}
      </span>
      <span className="flex shrink-0 items-center">
        {busy ? (
          <Button
            variant="outline"
            size="icon-sm"
            disabled
            className="shrink-0"
            aria-label={t(
              update.isPending
                ? "settings.plugins.updating"
                : "settings.plugins.installing",
            )}
          >
            <IconProgressDown />
          </Button>
        ) : installed === undefined ? (
          <Button
            variant="outline"
            size="icon-sm"
            className="shrink-0"
            onClick={() => install.mutate({}, { onError: failInstall })}
            aria-label={t("settings.plugins.install")}
          >
            <IconPlus />
          </Button>
        ) : hasUpdate ? (
          <Button
            variant="outline"
            size="icon-sm"
            className="shrink-0"
            aria-label={t("settings.plugins.update")}
            onClick={() => update.mutate({}, { onError: failUpdate })}
          >
            <IconArrowBigUpLines />
          </Button>
        ) : (
          <Button
            variant="outline"
            size="icon-sm"
            disabled
            className="shrink-0"
            aria-label={t("settings.plugins.installed")}
          >
            <IconCircleCheck />
          </Button>
        )}
      </span>
    </div>
  );
}
