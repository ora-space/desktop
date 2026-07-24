import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Button,
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  Input,
  Tabs,
  TabsList,
  TabsTrigger,
} from "@ora/ui";
import {
  IconChevronDown,
  IconChevronUp,
  IconDots,
  IconFilter,
  IconInfoCircle,
  IconSearch,
  IconSettings,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import {
  DEFAULT_INSTALLED_PLUGIN_IDS,
  PLUGIN_CATALOG,
  findPlugin,
  type PluginCollection,
  type PluginEntry,
} from "./plugin-catalog";
import { PluginDetail } from "./plugin-detail";
import { PluginManager } from "./plugin-manager";
import { PluginTile } from "./plugin-tile";
import { SettingsHeading } from "./settings-heading";

/** The number of leading names named in the collapsed "show more" row. */
const NAMED_IN_SHOW_MORE = 2;

/**
 * Beyond this many installed plugins the strip truncates and offers the manager instead.
 * Seven tiles plus the overflow tile is what fits on one line inside the settings pane's
 * ~704px content column (76px per tile, 8px gaps).
 */
const MAX_INSTALLED_TILES = 7;

/**
 * The plugin marketplace pane: an installed strip, a public/personal browse grid and a
 * per-plugin detail page. Nothing here talks to the backend — the catalog is hard-coded
 * and install state lives in component state, so installs reset when settings close.
 */
export function PluginsSettings() {
  const { t } = useTranslation();
  const [installedIds, setInstalledIds] = useState<string[]>(DEFAULT_INSTALLED_PLUGIN_IDS);
  const [query, setQuery] = useState("");
  const [collection, setCollection] = useState<PluginCollection>("public");
  const [expanded, setExpanded] = useState(false);
  const [openId, setOpenId] = useState<string | null>(null);
  const [managing, setManaging] = useState(false);
  const [disabledIds, setDisabledIds] = useState<string[]>([]);

  const toggleInstall = (id: string) => setInstalledIds((ids) => (
    ids.includes(id) ? ids.filter((current) => current !== id) : [...ids, id]
  ));
  const toggleEnabled = (id: string) => setDisabledIds((ids) => (
    ids.includes(id) ? ids.filter((current) => current !== id) : [...ids, id]
  ));

  const installed = useMemo(() => PLUGIN_CATALOG.filter((plugin) => installedIds.includes(plugin.id)), [installedIds]);

  const needle = query.trim().toLowerCase();
  const visible = useMemo(() => PLUGIN_CATALOG.filter((plugin) => plugin.collection === collection
    && (!needle
      || plugin.name.toLowerCase().includes(needle)
      || plugin.publisher.toLowerCase().includes(needle)
      || t(plugin.summaryKey).toLowerCase().includes(needle))), [collection, needle, t]);

  const openPlugin = openId === null ? undefined : findPlugin(openId);
  if (openPlugin) {
    return (
      <PluginDetail
        plugin={openPlugin}
        installed={installedIds.includes(openPlugin.id)}
        onBack={() => setOpenId(null)}
        onToggleInstall={() => toggleInstall(openPlugin.id)}
      />
    );
  }

  if (managing) {
    return (
      <PluginManager
        plugins={installed}
        disabledIds={disabledIds}
        onBack={() => setManaging(false)}
        onOpen={setOpenId}
        onToggleEnabled={toggleEnabled}
        onUninstall={toggleInstall}
      />
    );
  }

  // A search collapses the featured/rest split into one flat result list.
  const searching = needle.length > 0;
  const featured = visible.filter((plugin) => plugin.featured);
  const rest = visible.filter((plugin) => !plugin.featured);
  const collapsible = rest.length > NAMED_IN_SHOW_MORE;
  const grid = { installedIds, onOpen: setOpenId, onToggleInstall: toggleInstall };

  return (
    <div className="space-y-6">
      <SettingsHeading title={t("settings.plugins.title")} description={t("settings.plugins.description")} />

      <div className="relative">
        <IconSearch className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("settings.plugins.search")}
          aria-label={t("settings.plugins.search")}
          className="h-10 pl-9 pr-10"
        />
        {needle.length > 0 && (
          <Button
            variant="ghost"
            size="icon-sm"
            aria-label={t("settings.plugins.clearSearch")}
            className="absolute right-1.5 top-1/2 -translate-y-1/2 text-muted-foreground"
            onClick={() => setQuery("")}
          >
            <IconX />
          </Button>
        )}
      </div>

      {needle.length === 0 && (
        <section>
          <div className="flex h-8 items-center justify-between border-b border-border">
            <h3 className="text-sm font-medium">{t("settings.plugins.installed")}</h3>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label={t("settings.plugins.manageInstalled")}
              className="text-muted-foreground"
              onClick={() => setManaging(true)}
            >
              <IconSettings />
            </Button>
          </div>
          {/* Never wraps: the overflow tile is the row's last cell, not a second line. */}
          <div className="flex flex-nowrap items-start gap-x-2 overflow-hidden pt-2">
            {installed.length === 0 && <p className="py-4 text-sm text-muted-foreground">{t("settings.plugins.noneInstalled")}</p>}
            {installed.slice(0, MAX_INSTALLED_TILES).map((plugin) => (
              <InstalledTile key={plugin.id} plugin={plugin} onOpen={() => setOpenId(plugin.id)} />
            ))}
            {installed.length > MAX_INSTALLED_TILES && (
              <InstalledOverflowTile hidden={installed.length - MAX_INSTALLED_TILES} total={installed.length} onOpen={() => setManaging(true)} />
            )}
          </div>
        </section>
      )}

      <div className="flex items-center justify-between gap-3 border-b border-border pb-3">
        <Tabs
          value={collection}
          onValueChange={(value) => {
            setCollection(value as PluginCollection);
            setExpanded(false);
          }}
        >
          <TabsList>
            <TabsTrigger value="public">{t("settings.plugins.public")}</TabsTrigger>
            <TabsTrigger value="personal">{t("settings.plugins.personal")}</TabsTrigger>
          </TabsList>
        </Tabs>
        {/* Placeholder: the filter affordance is drawn but carries no behaviour yet. */}
        <Button variant="ghost" size="icon-sm" aria-label={t("settings.plugins.filter")} className="shrink-0 text-muted-foreground">
          <IconFilter />
        </Button>
      </div>

      {visible.length === 0 && <p className="py-10 text-center text-sm text-muted-foreground">{t("settings.plugins.empty")}</p>}

      {visible.length > 0 && searching && (
        <section className="space-y-2">
          <h3 className="flex items-baseline gap-2 text-sm font-medium">
            {t(collection === "public" ? "settings.plugins.public" : "settings.plugins.personal")}
            <span className="font-normal text-muted-foreground">{visible.length}</span>
          </h3>
          <PluginGrid items={visible} {...grid} />
        </section>
      )}

      {visible.length > 0 && !searching && (
        <>
          {featured.length > 0 && (
            <section className="space-y-2">
              <h3 className="text-sm font-medium">{t("settings.plugins.featured")}</h3>
              <PluginGrid items={featured} {...grid} />
            </section>
          )}
          {rest.length > 0 && (!collapsible || expanded) && (
            <section className="space-y-2">
              <h3 className="text-sm font-medium">{t("settings.plugins.more")}</h3>
              <PluginGrid items={rest} {...grid} />
              {collapsible && (
                <Button variant="ghost" size="sm" className="text-muted-foreground" onClick={() => setExpanded(false)}>
                  {t("settings.plugins.showLess")}
                  <IconChevronUp />
                </Button>
              )}
            </section>
          )}
          {collapsible && !expanded && <ShowMoreRow plugins={rest} onExpand={() => setExpanded(true)} />}
        </>
      )}
    </div>
  );
}

/** An installed plugin in the icon strip. The name stays visible; hovering lifts the tile. */
function InstalledTile({ plugin, onOpen }: { plugin: PluginEntry; onOpen: () => void }) {
  return (
    <button
      type="button"
      onClick={onOpen}
      title={plugin.name}
      className="group flex w-[76px] shrink-0 flex-col items-center gap-1.5 rounded-lg pt-1.5 outline-none"
    >
      <PluginTile
        plugin={plugin}
        size="lg"
        className="transition-transform duration-200 group-hover:-translate-y-1 group-focus-visible:-translate-y-1 group-focus-visible:ring-2 group-focus-visible:ring-ring"
      />
      <span className="w-full truncate text-center text-[11px] leading-4 text-muted-foreground transition-colors group-hover:text-foreground">
        {plugin.name}
      </span>
    </button>
  );
}

/**
 * Closes the installed strip once it overflows. It keeps an installed tile's exact
 * footprint — square mark, label underneath — so the row stays a single even grid
 * instead of wrapping onto a second line.
 */
function InstalledOverflowTile({ hidden, total, onOpen }: { hidden: number; total: number; onOpen: () => void }) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      onClick={onOpen}
      title={t("settings.plugins.viewAllInstalled", { count: total })}
      className="group flex w-[76px] shrink-0 flex-col items-center gap-1.5 rounded-lg pt-1.5 outline-none"
    >
      <span className="flex size-11 shrink-0 items-center justify-center text-base font-medium text-muted-foreground transition-all duration-200 group-hover:-translate-y-1 group-hover:text-foreground group-focus-visible:-translate-y-1">
        +{hidden}
      </span>
      <span className="w-full truncate text-center text-[11px] leading-4 text-muted-foreground transition-colors group-hover:text-foreground">
        {t("settings.plugins.viewAll")}
      </span>
    </button>
  );
}

/** Two-column browse grid shared by the featured, expanded and search result sections. */
function PluginGrid({ items, installedIds, onOpen, onToggleInstall }: {
  items: PluginEntry[];
  installedIds: string[];
  onOpen: (id: string) => void;
  onToggleInstall: (id: string) => void;
}) {
  return (
    <div className="grid gap-x-6 sm:grid-cols-2">
      {items.map((plugin) => (
        <PluginCard
          key={plugin.id}
          plugin={plugin}
          installed={installedIds.includes(plugin.id)}
          onOpen={() => onOpen(plugin.id)}
          onToggleInstall={() => onToggleInstall(plugin.id)}
        />
      ))}
    </div>
  );
}

/** One catalog row: the mark and copy open the detail page, the trailing control installs it. */
function PluginCard({ plugin, installed, onOpen, onToggleInstall }: {
  plugin: PluginEntry;
  installed: boolean;
  onOpen: () => void;
  onToggleInstall: () => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="flex items-center gap-3 rounded-lg p-2 transition-colors hover:bg-muted/50">
      <button
        type="button"
        onClick={onOpen}
        className="flex min-w-0 flex-1 items-center gap-3 rounded-md text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <PluginTile plugin={plugin} />
        <span className="min-w-0">
          <span className="block truncate text-sm font-medium">{plugin.name}</span>
          <span className="block truncate text-xs text-muted-foreground">{t(plugin.summaryKey)}</span>
          <span className="mt-0.5 block truncate text-[11px] text-muted-foreground/80">{plugin.publisher}</span>
        </span>
      </button>
      {installed
        ? <PluginActionsMenu plugin={plugin} onOpen={onOpen} onUninstall={onToggleInstall} />
        : <Button variant="outline" size="sm" className="shrink-0" onClick={onToggleInstall}>{t("settings.plugins.install")}</Button>}
    </div>
  );
}

/** The overflow menu shown in place of the install button once a plugin is installed. */
function PluginActionsMenu({ plugin, onOpen, onUninstall }: { plugin: PluginEntry; onOpen: () => void; onUninstall: () => void }) {
  const { t } = useTranslation();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={<Button variant="ghost" size="icon-sm" aria-label={t("settings.plugins.openMenu", { name: plugin.name })} className="shrink-0 text-muted-foreground" />}
      >
        <IconDots />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-44">
        <DropdownMenuItem onClick={onOpen}><IconInfoCircle />{t("settings.plugins.viewDetails")}</DropdownMenuItem>
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onClick={onUninstall}><IconTrash />{t("settings.plugins.uninstall")}</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** Collapsed tail of the catalog, previewing the remaining plugins as stacked marks. */
function ShowMoreRow({ plugins, onExpand }: { plugins: PluginEntry[]; onExpand: () => void }) {
  const { t } = useTranslation();
  const names = plugins.slice(0, NAMED_IN_SHOW_MORE).map((plugin) => plugin.name).join(", ");
  return (
    <button
      type="button"
      onClick={onExpand}
      className="flex w-full items-center gap-3 rounded-lg p-2 text-left outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring"
    >
      {/* Bare marks cannot overlap legibly, so the preview sits in a row rather than a stack. */}
      <span className="flex shrink-0 gap-1">
        {plugins.slice(0, 3).map((plugin) => <PluginTile key={plugin.id} plugin={plugin} size="sm" />)}
      </span>
      <span className="min-w-0 flex-1 truncate text-sm text-muted-foreground">
        {t("settings.plugins.showMore", { names, count: plugins.length - NAMED_IN_SHOW_MORE })}
      </span>
      <IconChevronDown className="size-4 shrink-0 text-muted-foreground" />
    </button>
  );
}
