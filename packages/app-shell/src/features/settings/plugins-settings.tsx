import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import type {
  AvailablePlugin,
  InstalledPlugin,
  InstallOutcome,
} from "@ora/contracts";
import {
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
  IconCheck,
  IconDownload,
  IconLoader2,
  IconProgressDown,
  IconRefresh,
  IconSearch,
  IconSettings,
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
import { PluginReadmeView } from "./plugin-readme-view";
import { PluginConfigurationEditor } from "./plugin-configuration-editor";
import type { PluginConfigurationNavigationGuard } from "./plugin-configuration-editor";

/** The registry kind order shown in the marketplace, mirroring the contracts docs. */
const MARKETPLACE_KIND_ORDER = [
  "agent",
  "workbench",
  "webview",
  "skill",
  "mcp",
  "hook",
];

/** Readable marketplace section labels for the known plugin kinds. */
const MARKETPLACE_KIND_LABELS: Record<string, string> = {
  agent: "Agent",
  workbench: "Workbench",
  webview: "Webview",
  skill: "Skill",
  mcp: "MCP",
  hook: "Hook",
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
  const [readmePlugin, setReadmePlugin] = useState<AvailablePlugin | null>(
    null,
  );

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
          onSuccess: (response) =>
            toast.success(
              installOutcomeMessage(
                response.outcome,
                t,
                "settings.plugins.importSuccess",
              ),
            ),
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

  if (readmePlugin !== null) {
    return (
      <PluginReadmeView
        plugin={readmePlugin}
        onBack={() => setReadmePlugin(null)}
      />
    );
  }

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
        onImport={() => void handleImport()}
        importing={importPlugin.isPending || selecting}
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
        <div className="flex items-center gap-1.5">
          <h2 className="text-lg font-semibold">
            {t("settings.plugins.title")}
          </h2>
          <DropdownMenu>
            <DropdownMenuTrigger
              aria-label={t("settings.plugins.manageActions")}
              className="flex size-7 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring data-popup-open:bg-accent data-popup-open:text-foreground"
            >
              <IconSettings className="size-4" />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start" className="w-44">
              <DropdownMenuItem onClick={() => setManaging(true)}>
                {t("settings.plugins.manageInstalled")}
              </DropdownMenuItem>
              <DropdownMenuItem onClick={() => setManagingSources(true)}>
                {t("settings.plugins.manageSources")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
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
            variant="ghost"
            size="sm"
            className="shrink-0 min-w-32"
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

        <span className="block text-xs text-muted-foreground">
          {lastSynced}
        </span>
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
                    onSelect={setReadmePlugin}
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
  onSelect,
}: {
  plugin: AvailablePlugin;
  installed: InstalledPlugin | undefined;
  onSelect: (plugin: AvailablePlugin) => void;
}) {
  const { t } = useTranslation();
  const install = useInstallPlugin(plugin.id);
  const update = useUpdatePlugin(plugin.id);
  const downloadPercentage =
    install.progress?.total !== null &&
    install.progress?.total !== undefined &&
    install.progress.total > 0
      ? Math.min(
          100,
          Math.round(
            (install.progress.downloaded / install.progress.total) * 100,
          ),
        )
      : null;
  const hasUpdate = plugin.version !== installed?.version;
  const incompatible = plugin.compatibility === "incompatible";

  const failInstall = (cause: unknown) => {
    toast.error(t("settings.plugins.installFailed"), {
      description: localizeContractError(cause, t),
    });
  };
  const succeedInstall = (response: { outcome: InstallOutcome }) => {
    toast.success(
      installOutcomeMessage(
        response.outcome,
        t,
        "settings.plugins.installSuccess",
      ),
    );
  };
  const failUpdate = (cause: unknown) => {
    toast.error(t("settings.plugins.updateFailed"), {
      description: localizeContractError(cause, t),
    });
  };

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={t("settings.plugins.viewReadme", {
        title: plugin.title || plugin.name,
      })}
      onClick={() => onSelect(plugin)}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          onSelect(plugin);
        }
      }}
      className="flex cursor-pointer items-center gap-3 rounded-lg border border-border p-3 outline-none transition-colors hover:bg-accent/50 focus-visible:ring-2 focus-visible:ring-ring"
    >
      <PluginLogo logo={plugin.logo} />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">
          {plugin.title || plugin.name}
        </span>
        {plugin.description !== "" && (
          <span className="mt-0.5 block truncate text-xs text-muted-foreground">
            {plugin.description}
          </span>
        )}
        {incompatible && (
          <span className="mt-0.5 block text-xs text-muted-foreground">
            {plugin.reason}
          </span>
        )}
      </span>
      <span className="flex shrink-0 items-center">
        {install.isPending ? (
          <Button
            variant="ghost"
            size="icon"
            disabled
            className="shrink-0 disabled:opacity-100"
            aria-label={t("settings.plugins.installing")}
          >
            <CircularDownloadProgress
              value={downloadPercentage}
              label={t("settings.plugins.downloadProgress")}
            />
          </Button>
        ) : update.isPending ? (
          <Button
            variant="ghost"
            size="icon"
            disabled
            className="shrink-0"
            aria-label={t("settings.plugins.updating")}
          >
            <IconProgressDown />
          </Button>
        ) : installed === undefined ? (
          <Button
            variant="outline"
            size="icon"
            className="shrink-0"
            disabled={incompatible}
            aria-label={t("settings.plugins.install")}
            onClick={(event) => {
              event.stopPropagation();
              install.mutate(
                {},
                { onError: failInstall, onSuccess: succeedInstall },
              );
            }}
          >
            <IconDownload />
          </Button>
        ) : hasUpdate ? (
          <Button
            variant="ghost"
            size="icon"
            className="shrink-0"
            aria-label={t("settings.plugins.update")}
            onClick={(event) => {
              event.stopPropagation();
              update.mutate({}, { onError: failUpdate });
            }}
          >
            <IconArrowBigUpLines />
          </Button>
        ) : (
          <Button
            variant="ghost"
            size="icon"
            disabled
            className="shrink-0"
            aria-label={t("settings.plugins.installed")}
          >
            <CompletedInstallIcon
              animate={install.completionId !== null}
              onAnimationComplete={install.consumeCompletion}
            />
          </Button>
        )}
      </span>
    </div>
  );
}

/** Matches the download ring's footprint and lets a newly installed check spring into place. */
function CompletedInstallIcon({
  animate,
  onAnimationComplete,
}: {
  animate: boolean;
  onAnimationComplete: () => void;
}) {
  return (
    <span
      data-slot="plugin-install-complete"
      data-animated={animate}
      className="relative grid size-6 place-items-center"
    >
      <span
        className={
          animate
            ? "absolute inset-0 rounded-full border-2 border-current animate-in fade-in-0 zoom-in-75 duration-200 motion-reduce:animate-none"
            : "absolute inset-0 rounded-full border-2 border-current"
        }
      />
      <IconCheck
        onAnimationEnd={animate ? onAnimationComplete : undefined}
        className={
          animate
            ? "size-3.5 stroke-[2.5] animate-in fade-in-0 zoom-in-0 delay-100 duration-300 fill-mode-both ease-[cubic-bezier(0.34,1.56,0.64,1)] motion-reduce:animate-none"
            : "size-3.5 stroke-[2.5]"
        }
      />
    </span>
  );
}

/** Renders determinate or indeterminate package progress inside the marketplace action button. */
function CircularDownloadProgress({
  value,
  label,
}: {
  value: number | null;
  label: string;
}) {
  const radius = 11;
  const circumference = 2 * Math.PI * radius;
  const offset =
    value === null ? 0 : circumference * (1 - Math.max(0, value) / 100);

  return (
    <span
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={value ?? undefined}
      className="relative grid size-7 place-items-center"
    >
      <svg
        viewBox="0 0 28 28"
        aria-hidden="true"
        className={
          value === null
            ? "size-7 -rotate-90 animate-spin"
            : "size-7 -rotate-90"
        }
      >
        <circle
          cx="14"
          cy="14"
          r={radius}
          fill="none"
          strokeWidth="2.5"
          className="stroke-muted"
        />
        <circle
          cx="14"
          cy="14"
          r={radius}
          fill="none"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeDasharray={
            value === null ? `18 ${circumference}` : circumference
          }
          strokeDashoffset={offset}
          className="stroke-primary transition-[stroke-dashoffset] duration-300 ease-out"
        />
      </svg>
    </span>
  );
}

/** Maps a typed install outcome to the toast the settings surface already shows. */
function installOutcomeMessage(
  outcome: InstallOutcome,
  t: TFunction,
  successKey:
    "settings.plugins.installSuccess" | "settings.plugins.importSuccess",
): string {
  if (outcome.state === "installed_with_command_conflict") {
    return t("settings.plugins.installCommandConflict", {
      pluginId: outcome.conflictPluginId,
    });
  }
  return t(successKey);
}
