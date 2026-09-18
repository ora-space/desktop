import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import type {
  AvailablePlugin,
  InstalledPlugin,
  PackInstallationStatus,
  PackMemberReconciliationState,
} from "@ora/contracts";
import {
  Badge,
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
  IconRefresh,
  IconSearch,
  IconSettings,
} from "@tabler/icons-react";
import { useContractErrorToast } from "../../i18n/use-contract-error-toast";
import { useContractsClient } from "../../contracts-client-context";
import {
  invalidateInstalledPlugins,
  invalidatePackInstallations,
} from "../../state/data/plugins";
import { usePlatform } from "../../platform";
import { useAvailablePlugins } from "../../state/hooks/use-available-plugins";
import { PackUninstallConfirm } from "./pack-uninstall-confirm";
import { usePackInstallations } from "../../state/hooks/use-pack-installations";
import { useInstallPlugin } from "../../state/hooks/use-install-plugin";
import { useUpdatePlugin } from "../../state/hooks/use-update-plugin";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";
import { usePluginImport } from "../../state/hooks/use-plugin-import";
import { usePluginRegistrySync } from "../../state/hooks/use-plugin-registry-sync";
import { useMarketplaceSyncStore } from "../../state/stores/marketplace-sync-store";
import { PluginLogo } from "./plugin-logo";
import { PluginSourcesManager } from "./plugin-sources-manager";
import { PluginManager } from "./plugin-manager";
import { PluginReadmeView } from "./plugin-readme-view";
import { PluginConfigurationEditor } from "./plugin-configuration-editor";
import type { PluginConfigurationNavigationGuard } from "./plugin-configuration-editor";
import { PluginDownloadProgress } from "./plugin-download-progress";
import { showPluginInstallOutcome } from "./plugin-install-feedback";
import {
  HookExecutionConfirm,
  type HookExecutionAction,
} from "./hook-execution-confirm";
import { useUiStore } from "../../state/stores/ui-store";

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
  detailPluginId = null,
  onDetailClose,
}: {
  onNavigationGuardChange?: (
    guard: PluginConfigurationNavigationGuard | null,
  ) => void;
  detailPluginId?: string | null;
  onDetailClose?: () => void;
}) {
  const { t } = useTranslation();
  const showContractError = useContractErrorToast();
  // Another surface may deep-link here (e.g. a workflow dependency to install or
  // configure). Adopt the request once so later visits start from the default view.
  const [initialRequest] = useState(
    () => useUiStore.getState().pluginSettingsRequest,
  );
  const clearPluginSettingsRequest = useUiStore(
    (state) => state.clearPluginSettingsRequest,
  );
  useEffect(() => {
    clearPluginSettingsRequest();
  }, [clearPluginSettingsRequest]);
  const [query, setQuery] = useState(
    initialRequest?.kind === "marketplaceSearch" ? initialRequest.query : "",
  );
  const [managing, setManaging] = useState(
    initialRequest?.kind === "manage" || initialRequest?.kind === "configure",
  );
  const [managingSources, setManagingSources] = useState(false);
  const [configurationPlugin, setConfigurationPlugin] = useState<{
    id: string;
    displayName: string;
  } | null>(
    initialRequest?.kind === "configure"
      ? {
          id: initialRequest.pluginId,
          displayName: initialRequest.displayName,
        }
      : null,
  );
  const [selecting, setSelecting] = useState(false);
  const [readmePlugin, setReadmePlugin] = useState<AvailablePlugin | null>(
    null,
  );
  const [uninstallingPack, setUninstallingPack] = useState<string | null>(null);
  const packInstallations = usePackInstallations();
  const queryClient = useQueryClient();
  const client = useContractsClient();
  const uninstallPackMutation = useMutation({
    // A pack removal never runs a member's lifecycle commands, so it authorizes nothing: the
    // declaration stays false even though the user did confirm the removal.
    mutationFn: (packId: string) =>
      client.plugin.uninstall({
        pluginId: packId,
        dataDisposition: "delete" as const,
        hookExecutionAcknowledged: false,
      }),
    onSuccess: () => {
      toast.success(t("settings.plugins.packUninstallSuccess"));
      setUninstallingPack(null);
    },
    onError: (cause) => {
      showContractError(cause, t("settings.plugins.uninstallFailed"));
    },
    onSettled: async () => {
      await invalidatePackInstallations(queryClient);
      await invalidateInstalledPlugins(queryClient);
    },
  });

  const platform = usePlatform();
  const available = useAvailablePlugins();
  const installed = useInstalledPlugins();
  const sync = usePluginRegistrySync();
  const importPlugin = usePluginImport();
  // The host admits one rebuild at a time and discards the rest, so a click made while its own
  // refresh is running would be dropped rather than served. The action stands down instead.
  // `sync.isPending` covers the user's own sync and, unlike the mutation, survives this page
  // being left and reopened mid-sync.
  const hostRefreshing = useMarketplaceSyncStore(
    (state) => state.hostRefreshing,
  );
  const syncing = sync.isPending || hostRefreshing;

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
            plugin.sourceUrl,
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
            showPluginInstallOutcome(
              response.outcome,
              t,
              "settings.plugins.importSuccess",
            ),
          onError: (cause) =>
            showContractError(cause, t("settings.plugins.importFailed")),
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

  const detailPlugin =
    (detailPluginId === null ? undefined : availableById.get(detailPluginId)) ??
    readmePlugin;

  if (detailPlugin !== null && detailPlugin !== undefined) {
    return (
      <PluginReadmeView
        plugin={detailPlugin}
        installed={installedById.get(detailPlugin.id)}
        onBack={() => {
          setReadmePlugin(null);
          onDetailClose?.();
        }}
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
            disabled={syncing}
            onClick={() =>
              sync.mutate(undefined, {
                onError: (cause) => {
                  showContractError(cause, t("settings.plugins.syncFailed"));
                },
              })
            }
            aria-label={t("settings.plugins.syncMarketplace")}
          >
            {syncing ? (
              <IconLoader2 className="animate-spin" />
            ) : (
              <IconRefresh />
            )}
            <span className="hidden sm:inline">
              {syncing
                ? t("settings.plugins.syncingMarketplace")
                : t("settings.plugins.syncMarketplace")}
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

      <InstalledPacksSection
        packs={packInstallations.data ?? []}
        availableById={availableById}
        onUninstall={(packId) => setUninstallingPack(packId)}
      />

      {uninstallingPack !== null && (
        <PackUninstallConfirm
          packId={uninstallingPack}
          open
          onOpenChange={(open) => {
            if (!open) setUninstallingPack(null);
          }}
          onConfirm={() => {
            uninstallPackMutation.mutate(uninstallingPack);
          }}
          busy={uninstallPackMutation.isPending}
        />
      )}
    </div>
  );
}

/**
 * The installed-packs presentation is sourced from the ownership journal plus its
 * reconciliation — packs are never faked into the installed-plugin directory (extension-pack
 * decision D7 / D3-A).
 */
function InstalledPacksSection({
  packs,
  availableById,
  onUninstall,
}: {
  packs: PackInstallationStatus[];
  availableById: Map<string, AvailablePlugin>;
  onUninstall: (packId: string) => void;
}) {
  const { t } = useTranslation();
  if (packs.length === 0) return null;

  return (
    <section>
      <h3 className="mb-2 text-sm font-semibold">
        {t("settings.plugins.packsSection")}
      </h3>
      <div className="space-y-3">
        {packs.map((pack) => {
          const listing = availableById.get(pack.packId);
          return (
            <div
              key={pack.packId}
              className="rounded-lg border border-border p-3"
            >
              <div className="flex items-center gap-2">
                <span className="text-sm font-medium">
                  {listing?.title ?? pack.packId}
                </span>
                <span className="text-xs text-muted-foreground">
                  {t("settings.plugins.packMembersCount", {
                    count: pack.members.length,
                  })}
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  className="ml-auto"
                  onClick={() => onUninstall(pack.packId)}
                >
                  {t("settings.plugins.packUninstall")}
                </Button>
              </div>
              <ul className="mt-2 space-y-1">
                {pack.members.map((member) => (
                  <li
                    key={member.memberId}
                    className="flex items-center justify-between text-xs"
                  >
                    <span className="truncate text-muted-foreground">
                      {member.memberId}
                      {member.ownership === "pre_existing" &&
                        ` · ${t("settings.plugins.packMemberPreExisting")}`}
                    </span>
                    <PackMemberStateBadge state={member.state} />
                  </li>
                ))}
              </ul>
            </div>
          );
        })}
      </div>
    </section>
  );
}

/** Presents one reconciled pack member state as a restrained badge. */
function PackMemberStateBadge({
  state,
}: {
  state: PackMemberReconciliationState;
}) {
  const { t } = useTranslation();
  const label =
    state.state === "expected_and_present"
      ? t("settings.plugins.packMemberExpected")
      : state.state === "version_changed"
        ? t("settings.plugins.packMemberVersionChanged", {
            version: state.currentVersion,
          })
        : t("settings.plugins.packMemberMissing");
  const destructive =
    state.state === "missing" || state.state === "version_changed";
  return (
    <Badge variant={destructive ? "destructive" : "secondary"}>{label}</Badge>
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
  const showContractError = useContractErrorToast();
  const install = useInstallPlugin(plugin.id);
  const update = useUpdatePlugin(plugin.id);
  const hasUpdate = plugin.version !== installed?.version;
  const incompatible = plugin.compatibility === "incompatible";
  const isHook = plugin.kind === "hook";
  const [confirmAction, setConfirmAction] =
    useState<HookExecutionAction | null>(null);

  const failInstall = (cause: unknown) => {
    showContractError(cause, t("settings.plugins.installFailed"));
  };
  const succeedInstall = (response: {
    outcome: Parameters<typeof showPluginInstallOutcome>[0];
  }) => showPluginInstallOutcome(response.outcome, t);
  const failUpdate = (cause: unknown) => {
    showContractError(cause, t("settings.plugins.updateFailed"));
  };
  /** Asks for the Hook execution disclosure before the one action that would run a program. */
  const start = (action: HookExecutionAction) => {
    if (isHook) {
      setConfirmAction(action);
      return;
    }
    if (action === "install") {
      install.mutate({}, { onError: failInstall, onSuccess: succeedInstall });
      return;
    }
    update.mutate({}, { onError: failUpdate });
  };
  const confirm = (action: HookExecutionAction) => {
    setConfirmAction(null);
    if (action === "install") {
      install.mutate(
        { hookExecutionAcknowledged: true },
        { onError: failInstall, onSuccess: succeedInstall },
      );
      return;
    }
    update.mutate({ hookExecutionAcknowledged: true }, { onError: failUpdate });
  };

  return (
    <>
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
          {plugin.packMembers !== null && plugin.packMembers !== undefined && (
            <span className="mt-0.5 block truncate text-xs text-muted-foreground">
              {t("settings.plugins.packMembersCount", {
                count: plugin.packMembers.length,
              })}
              {": "}
              {plugin.packMembers.join(", ")}
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
              <PluginDownloadProgress
                progress={install.progress}
                label={t("settings.plugins.downloadProgress")}
              />
            </Button>
          ) : update.isPending ? (
            <Button
              variant="ghost"
              size="icon"
              disabled
              className="shrink-0 disabled:opacity-100"
              aria-label={t("settings.plugins.updating")}
            >
              <PluginDownloadProgress
                progress={update.progress}
                label={t("settings.plugins.downloadProgress")}
              >
                <IconArrowBigUpLines className="size-3.5" />
              </PluginDownloadProgress>
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
                start("install");
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
                start("update");
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
      {/*
        Sits beside the card rather than inside it: the card is one big button, and React portals
        still bubble their events along the React tree, so a dialog rendered within it would open
        the detail page as soon as the user answered the confirmation.
      */}
      {isHook && (
        <HookExecutionConfirm
          name={plugin.title || plugin.name}
          action={confirmAction ?? "install"}
          open={confirmAction !== null}
          onOpenChange={(open) => setConfirmAction(open ? "install" : null)}
          onConfirm={() => confirm(confirmAction ?? "install")}
          busy={install.isPending || update.isPending}
        />
      )}
    </>
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
