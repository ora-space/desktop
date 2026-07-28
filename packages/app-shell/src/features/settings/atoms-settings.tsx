import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { Agent, Skill } from "@ora/contracts";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Textarea,
  cn,
} from "@ora/ui";
import {
  IconActivity,
  IconBug,
  IconBuildingStore,
  IconCheck,
  IconCloud,
  IconCloudOff,
  IconPackage,
  IconPencil,
  IconPlus,
  IconRobot,
  IconSearch,
  IconShieldCheck,
  IconSparkles,
  IconStar,
  IconTrash,
  IconX,
} from "@tabler/icons-react";
import { useAgents } from "../../state/hooks/use-agents";
import { useSkills } from "../../state/hooks/use-skills";
import {
  useCreateAgent,
  useUpdateAgent,
  useDeleteAgent,
  useCreateSkill,
  useUpdateSkill,
  useDeleteSkill,
} from "../../state/hooks/use-atom-mutations";
import {
  CATALOG_AGENTS,
  CATALOG_SKILLS,
  COMMON_AGENT_IDS,
  COMMON_SKILL_IDS,
  SKILL_MARKET_ITEMS,
  isCatalogAtom,
  isInternalAtom,
  type SkillMarketCategory,
  type SkillMarketItem,
} from "./atom-catalog";
import { SettingsHeading } from "./settings-heading";

type AtomRecord = Agent | Skill;
type TablerIcon = typeof IconRobot;
type MarketplaceCategory = "all" | SkillMarketCategory;

const MARKETPLACE_CATEGORIES: MarketplaceCategory[] = [
  "all",
  "build",
  "cloud",
  "observability",
  "security",
  "diagnostics",
];

const MARKETPLACE_CATEGORY_ICONS: Record<SkillMarketCategory, TablerIcon> = {
  build: IconPackage,
  cloud: IconCloud,
  observability: IconActivity,
  security: IconShieldCheck,
  diagnostics: IconBug,
};

/** The i18n namespace and behaviour that distinguish the two atom panes. */
interface AtomManagerConfig {
  /** Translation key prefix, e.g. `settings.roles`. */
  tPrefix: string;
  /** Neutral mark drawn beside each row. */
  icon: TablerIcon;
  /** Roles carry an extra body field; skills do not. */
  hasBody: boolean;
  /** Restrained accent used to distinguish role and skill catalogs at a glance. */
  accentClassName: string;
  items: AtomRecord[];
  commonItemIds: readonly string[];
  remoteError: boolean;
  onOpenMarketplace?: () => void;
  onCreate: (name: string, description: string) => Promise<void>;
  onUpdate: (item: AtomRecord, name: string, description: string) => Promise<void>;
  onDelete: (item: AtomRecord) => Promise<void>;
}

/** The Roles pane manages the configurable agents surfaced to Ora sessions. */
export function RolesSettings() {
  const agentsQuery = useAgents();
  const createAgent = useCreateAgent();
  const updateAgent = useUpdateAgent();
  const deleteAgent = useDeleteAgent();
  const [catalogAgents, setCatalogAgents] = useState<Agent[]>(() => [...CATALOG_AGENTS]);

  return (
    <AtomManager
      tPrefix="settings.roles"
      icon={IconRobot}
      hasBody
      accentClassName="border-sky-500/20 bg-sky-500/10 text-sky-700 dark:text-sky-300"
      items={[...catalogAgents, ...(agentsQuery.data ?? [])]}
      commonItemIds={COMMON_AGENT_IDS}
      remoteError={agentsQuery.error !== null}
      onCreate={(name, description) => createAgent.mutateAsync({ name, description }).then(() => undefined)}
      onUpdate={(item, name, description) => {
        if (isCatalogAtom(item)) {
          setCatalogAgents((current) => current.map((agent) => (
            agent.id === item.id ? { ...agent, name, description } : agent
          )));
          return Promise.resolve();
        }
        return updateAgent.mutateAsync({ agent: item as Agent, name, description }).then(() => undefined);
      }}
      onDelete={(item) => {
        if (isCatalogAtom(item)) {
          setCatalogAgents((current) => current.filter((agent) => agent.id !== item.id));
          return Promise.resolve();
        }
        return deleteAgent.mutateAsync({ agentId: item.id }).then(() => undefined);
      }}
    />
  );
}

/** The Skills pane manages the reusable skills surfaced to Ora sessions. */
export function SkillsSettings() {
  const skillsQuery = useSkills();
  const createSkill = useCreateSkill();
  const updateSkill = useUpdateSkill();
  const deleteSkill = useDeleteSkill();
  const [catalogSkills, setCatalogSkills] = useState<Skill[]>(() => [...CATALOG_SKILLS]);
  const [marketplaceOpen, setMarketplaceOpen] = useState(false);
  const installedSkillIds = useMemo(() => new Set(catalogSkills.map(({ id }) => id)), [catalogSkills]);

  return (
    <>
      <AtomManager
        tPrefix="settings.skills"
        icon={IconSparkles}
        hasBody={false}
        accentClassName="border-amber-500/20 bg-amber-500/10 text-amber-700 dark:text-amber-300"
        items={[...catalogSkills, ...(skillsQuery.data ?? [])]}
        commonItemIds={COMMON_SKILL_IDS}
        remoteError={skillsQuery.error !== null}
        onOpenMarketplace={() => setMarketplaceOpen(true)}
        onCreate={(name, description) => createSkill.mutateAsync({ name, description }).then(() => undefined)}
        onUpdate={(item, name, description) => {
          if (isCatalogAtom(item)) {
            setCatalogSkills((current) => current.map((skill) => (
              skill.id === item.id ? { ...skill, name, description } : skill
            )));
            return Promise.resolve();
          }
          return updateSkill.mutateAsync({ skill: item as Skill, name, description }).then(() => undefined);
        }}
        onDelete={(item) => {
          if (isCatalogAtom(item)) {
            setCatalogSkills((current) => current.filter((skill) => skill.id !== item.id));
            return Promise.resolve();
          }
          return deleteSkill.mutateAsync({ skillId: item.id }).then(() => undefined);
        }}
      />
      <SkillMarketplaceDialog
        open={marketplaceOpen}
        installedIds={installedSkillIds}
        onOpenChange={setMarketplaceOpen}
        onInstall={(skill) => setCatalogSkills((current) => (
          current.some(({ id }) => id === skill.id) ? current : [...current, skill]
        ))}
      />
    </>
  );
}

/**
 * The list-and-editor surface shared by both panes. While creating or editing, the toolbar and
 * list are replaced entirely by {@link AtomEditor}; leaving the editor brings the list back.
 */
function AtomManager({
  tPrefix,
  icon,
  hasBody,
  accentClassName,
  items,
  commonItemIds,
  remoteError,
  onOpenMarketplace,
  onCreate,
  onUpdate,
  onDelete,
}: AtomManagerConfig) {
  const { t } = useTranslation();
  const CatalogIcon = icon;
  const [query, setQuery] = useState("");
  const [visibleLimit, setVisibleLimit] = useState(24);
  // `null` = list view; `{ item: null }` = creating; `{ item }` = editing that record.
  const [editing, setEditing] = useState<{ item: AtomRecord | null } | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<AtomRecord | null>(null);

  const needle = query.trim().toLowerCase();
  const visibleItems = useMemo(
    () => items.filter((item) => !needle
      || item.name.toLowerCase().includes(needle)
      || item.description.toLowerCase().includes(needle)),
    [needle, items],
  );
  const commonItems = useMemo(
    () => commonItemIds.flatMap((id) => {
      const item = items.find((candidate) => candidate.id === id);
      return item ? [item] : [];
    }),
    [commonItemIds, items],
  );
  const renderedItems = visibleItems.slice(0, visibleLimit);

  const save = async (name: string, description: string) => {
    if (editing?.item) await onUpdate(editing.item, name, description);
    else await onCreate(name, description);
    setEditing(null);
  };

  if (editing !== null) {
    return (
      <div className="space-y-5">
        <SettingsHeading title={t(`${tPrefix}.title`)} description={t(`${tPrefix}.description`)} />
        <AtomEditor
          key={editing.item?.id ?? "new"}
          tPrefix={tPrefix}
          hasBody={hasBody}
          item={editing.item}
          onCancel={() => setEditing(null)}
          onSave={save}
        />
      </div>
    );
  }

  return (
    <div className="space-y-5">
      <SettingsHeading title={t(`${tPrefix}.title`)} description={t(`${tPrefix}.description`)} />

      <section className="overflow-hidden rounded-xl border border-border bg-card shadow-sm shadow-foreground/[0.025]">
        <div className="flex flex-col gap-3 border-b border-border bg-muted/20 p-3 sm:flex-row sm:items-center">
          <div className="relative min-w-0 flex-1">
            <IconSearch className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              value={query}
              onChange={(event) => {
                setQuery(event.target.value);
                setVisibleLimit(24);
              }}
              placeholder={t(`${tPrefix}.search`)}
              aria-label={t(`${tPrefix}.search`)}
              className="h-9 bg-background pl-9 pr-9"
            />
            {query && (
              <Button
                variant="ghost"
                size="icon-sm"
                className="absolute right-1 top-1/2 -translate-y-1/2 text-muted-foreground"
                aria-label={t(`${tPrefix}.clearSearch`)}
                onClick={() => {
                  setQuery("");
                  setVisibleLimit(24);
                }}
              >
                <IconX />
              </Button>
            )}
          </div>
          {onOpenMarketplace && (
            <Button variant="outline" size="sm" className="h-9 shrink-0" onClick={onOpenMarketplace}>
              <IconBuildingStore />{t(`${tPrefix}.marketplace`)}
            </Button>
          )}
          <Button variant="secondary" size="sm" className="h-9 shrink-0 shadow-sm" onClick={() => setEditing({ item: null })}>
            <IconPlus />{t(`${tPrefix}.new`)}
          </Button>
        </div>

        <div className="flex items-center justify-between gap-3 px-4 py-3">
          <div className="flex items-center gap-2">
            <div className={cn("flex size-8 items-center justify-center rounded-lg border", accentClassName)}>
              <CatalogIcon className="size-4" />
            </div>
            <div>
              <h3 className="text-xs font-semibold">{t(`${tPrefix}.sectionLabel`)}</h3>
              <p className="text-[11px] text-muted-foreground">{t(`${tPrefix}.catalogHint`)}</p>
            </div>
          </div>
          <p className="text-[11px] tabular-nums text-muted-foreground" aria-live="polite">
            {t(`${tPrefix}.resultCount`, { visible: visibleItems.length, total: items.length })}
          </p>
        </div>

        {!needle && commonItems.length > 0 && (
          <div className="border-b border-border px-3 pb-3">
            <div className="mb-2 flex items-center gap-1.5 px-1">
              <IconStar className="size-3.5 text-muted-foreground" />
              <h3 className="text-xs font-semibold">{t(`${tPrefix}.commonLabel`)}</h3>
              <span className="text-[11px] text-muted-foreground">{t(`${tPrefix}.commonHint`)}</span>
            </div>
            <div className="grid grid-cols-2 gap-2 md:grid-cols-4">
              {commonItems.map((item) => (
                <button
                  key={item.id}
                  type="button"
                  className="min-w-0 cursor-pointer rounded-lg border border-border bg-background px-2.5 py-2 text-left outline-none transition-colors duration-150 hover:border-foreground/20 hover:bg-muted/30 focus-visible:ring-2 focus-visible:ring-ring"
                  onClick={() => setEditing({ item })}
                >
                  <span className="flex min-w-0 items-center gap-1.5">
                    <span className="min-w-0 flex-1 truncate text-xs font-medium">{item.name}</span>
                    {isInternalAtom(item) && (
                      <span className="shrink-0 rounded-full border border-primary/20 bg-primary/5 px-1.5 py-0.5 text-[8px] font-semibold text-primary">
                        {t(`${tPrefix}.internalBadge`)}
                      </span>
                    )}
                  </span>
                  <span className="mt-0.5 block truncate text-[10px] text-muted-foreground">{item.description}</span>
                </button>
              ))}
            </div>
          </div>
        )}

        {remoteError && (
          <div className="mx-3 flex items-center gap-2 rounded-md border border-amber-500/20 bg-amber-500/5 px-3 py-2 text-[11px] text-muted-foreground">
            <IconCloudOff className="size-3.5 shrink-0 text-amber-600 dark:text-amber-400" />
            {t(`${tPrefix}.mockFallback`)}
          </div>
        )}

        {visibleItems.length === 0 ? (
          <div className="flex flex-col items-center px-4 py-12 text-center">
            <div className={cn("mb-3 flex size-10 items-center justify-center rounded-xl border", accentClassName)}>
              <IconSearch className="size-4" />
            </div>
            <p className="text-sm font-medium">{t(`${tPrefix}.emptySearchTitle`)}</p>
            <p className="mt-1 max-w-xs text-xs leading-5 text-muted-foreground">{t(`${tPrefix}.emptySearchDescription`, { query })}</p>
            <Button
              variant="ghost"
              size="sm"
              className="mt-3"
              onClick={() => {
                setQuery("");
                setVisibleLimit(24);
              }}
            >
              {t(`${tPrefix}.clearSearch`)}
            </Button>
          </div>
        ) : (
          <>
            <div className="grid grid-cols-1 gap-2.5 p-3 md:grid-cols-2">
              {renderedItems.map((item) => {
                const Icon = icon;
                const internal = isInternalAtom(item);
                return (
                  <article
                    key={item.id}
                    className="group flex min-h-32 flex-col rounded-xl border border-border bg-background p-3 outline-none transition-[border-color,box-shadow,transform] duration-150 hover:-translate-y-px hover:border-foreground/20 hover:shadow-md hover:shadow-foreground/5 focus-within:border-ring focus-within:ring-2 focus-within:ring-ring/20"
                  >
                    <div className="flex items-start gap-2">
                      <div className={cn("flex size-8 shrink-0 items-center justify-center rounded-lg border", accentClassName)}>
                        <Icon className="size-4" />
                      </div>
                      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1.5 pt-0.5">
                        {internal && (
                          <span className="rounded-full border border-primary/20 bg-primary/5 px-1.5 py-0.5 text-[9px] font-semibold text-primary">
                            {t(`${tPrefix}.internalBadge`)}
                          </span>
                        )}
                      </div>
                      <div className="flex shrink-0 gap-0.5">
                        <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("common.edit")} onClick={() => setEditing({ item })}><IconPencil /></Button>
                        <Button variant="ghost" size="icon-sm" className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive" aria-label={t("common.delete")} onClick={() => setDeleteTarget(item)}><IconTrash /></Button>
                      </div>
                    </div>
                    <div className="mt-3 min-w-0">
                      <h4 className="truncate text-sm font-semibold tracking-tight">{item.name}</h4>
                      <p className="mt-1 line-clamp-2 text-xs leading-5 text-muted-foreground">{item.description}</p>
                    </div>
                  </article>
                );
              })}
            </div>
            {renderedItems.length < visibleItems.length && (
              <div className="flex justify-center border-t border-border p-3">
                <Button variant="ghost" size="sm" onClick={() => setVisibleLimit((current) => current + 24)}>
                  {t(`${tPrefix}.showMore`, { count: Math.min(24, visibleItems.length - renderedItems.length) })}
                </Button>
              </div>
            )}
          </>
        )}
      </section>

      <DeleteAtomDialog tPrefix={tPrefix} target={deleteTarget} onOpenChange={(open) => !open && setDeleteTarget(null)} onDelete={onDelete} />
    </div>
  );
}

/** Borderless field styling so name and description read as inline text inside the card. */
const INLINE_FIELD = "border-transparent bg-transparent px-0 shadow-none focus-visible:border-transparent focus-visible:ring-0 dark:bg-transparent";

/**
 * The full-surface create/edit form. Name and description sit in a card with a label-left
 * layout; roles add a large borderless body editor below. The body and the "improve" button
 * are currently UI-only affordances that are intentionally not wired to the backend yet.
 */
function AtomEditor({ tPrefix, hasBody, item, onCancel, onSave }: {
  tPrefix: string;
  hasBody: boolean;
  item: AtomRecord | null;
  onCancel: () => void;
  onSave: (name: string, description: string) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(() => item?.name ?? "");
  const [description, setDescription] = useState(() => item?.description ?? "");
  const [body, setBody] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!name.trim() || !description.trim() || saving) return;
    setSaving(true);
    setError(null);
    try {
      await onSave(name.trim(), description.trim());
    } catch {
      setError(t(`${tPrefix}.saveError`));
      setSaving(false);
    }
  };

  return (
    <form onSubmit={(event) => void submit(event)} className="space-y-5">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-medium">{item ? t(`${tPrefix}.editTitle`) : t(`${tPrefix}.createTitle`)}</h3>
        <div className="flex items-center gap-2">
          <Button type="button" variant="ghost" size="sm" disabled={saving} onClick={onCancel}>{t("common.cancel")}</Button>
          <Button type="submit" variant="secondary" size="sm" disabled={saving || !name.trim() || !description.trim()}>{saving ? t("common.saving") : t("common.save")}</Button>
        </div>
      </div>

      <div className="rounded-xl border border-border bg-muted/20 p-5">
        <div className="divide-y divide-border/60">
          <div className="grid grid-cols-[72px_minmax(0,1fr)] items-center gap-4 pb-3">
            <Label htmlFor="atom-name" className="text-muted-foreground">{t(`${tPrefix}.nameLabel`)}</Label>
            <Input id="atom-name" value={name} onChange={(event) => setName(event.target.value)} placeholder={t(`${tPrefix}.namePlaceholder`)} autoFocus className={INLINE_FIELD} />
          </div>
          <div className="grid grid-cols-[72px_minmax(0,1fr)] items-start gap-4 pt-3">
            <Label htmlFor="atom-description" className="pt-1.5 text-muted-foreground">{t(`${tPrefix}.descriptionLabel`)}</Label>
            <Textarea id="atom-description" value={description} onChange={(event) => setDescription(event.target.value)} placeholder={t(`${tPrefix}.descriptionPlaceholder`)} className={cn(INLINE_FIELD, "min-h-9 resize-none py-1.5")} />
          </div>
        </div>
      </div>

      {hasBody && (
        <div className="space-y-1.5">
          <div className="rounded-xl border border-border bg-muted/20 p-4">
            <Textarea id="atom-body" value={body} onChange={(event) => setBody(event.target.value)} placeholder={t(`${tPrefix}.bodyPlaceholder`)} className={cn(INLINE_FIELD, "min-h-56 resize-none")} />
          </div>
          <p className="px-1 text-[11px] leading-4 text-muted-foreground">{t(`${tPrefix}.bodyHint`)}</p>
        </div>
      )}

      {error && <p className="text-xs text-destructive">{error}</p>}
    </form>
  );
}

/** Lets users add curated community capabilities without leaving the settings flow. */
function SkillMarketplaceDialog({
  open,
  installedIds,
  onOpenChange,
  onInstall,
}: {
  open: boolean;
  installedIds: ReadonlySet<string>;
  onOpenChange: (open: boolean) => void;
  onInstall: (skill: SkillMarketItem) => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<MarketplaceCategory>("all");
  const needle = query.trim().toLowerCase();
  const visibleSkills = useMemo(
    () => SKILL_MARKET_ITEMS.filter((skill) => (
      (category === "all" || skill.category === category)
      && (!needle
        || skill.name.toLowerCase().includes(needle)
        || skill.description.toLowerCase().includes(needle)
        || skill.publisher.toLowerCase().includes(needle))
    )),
    [category, needle],
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[min(760px,calc(100dvh-2rem))] w-[min(880px,calc(100vw-2rem))] max-w-none gap-0 overflow-hidden p-0 sm:max-w-none">
        <DialogHeader className="relative overflow-hidden border-b border-border bg-muted/25 px-5 py-5 pr-12 text-left">
          <div className="pointer-events-none absolute -right-12 -top-20 size-48 rounded-full bg-primary/10 blur-3xl" />
          <div className="pointer-events-none absolute -bottom-20 left-1/3 size-40 rounded-full bg-amber-500/10 blur-3xl" />
          <div className="relative">
            <p className="mb-3 flex items-center gap-1.5 text-[10px] font-semibold uppercase tracking-[0.18em] text-muted-foreground">
              <IconBuildingStore className="size-3.5" />
              {t("settings.skills.marketplaceRegistry")}
            </p>
            <DialogTitle className="text-xl font-semibold tracking-tight">
              {t("settings.skills.marketplaceTitle")}
            </DialogTitle>
            <DialogDescription className="mt-1.5 max-w-2xl text-xs leading-5">
              {t("settings.skills.marketplaceDescription")}
            </DialogDescription>
            <div className="mt-4 flex flex-wrap gap-x-5 gap-y-2 text-[11px] text-muted-foreground">
              <span className="flex items-center gap-1.5"><IconCheck className="size-3.5 text-emerald-600 dark:text-emerald-400" />{t("settings.skills.marketplaceCurated")}</span>
              <span className="flex items-center gap-1.5"><IconPackage className="size-3.5" />{t("settings.skills.marketplaceCount", { count: SKILL_MARKET_ITEMS.length })}</span>
              <span className="flex items-center gap-1.5"><IconActivity className="size-3.5" />{t("settings.skills.marketplaceInstant")}</span>
            </div>
          </div>
        </DialogHeader>

        <div className="min-h-0 overflow-y-auto p-4">
          <div className="flex flex-col gap-3">
            <div className="relative">
              <IconSearch className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
              <Input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={t("settings.skills.marketplaceSearch")}
                aria-label={t("settings.skills.marketplaceSearch")}
                className="h-10 bg-background pl-9 pr-9 shadow-sm"
              />
              {query && (
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="absolute right-1.5 top-1/2 -translate-y-1/2 text-muted-foreground"
                  aria-label={t("settings.skills.clearSearch")}
                  onClick={() => setQuery("")}
                >
                  <IconX />
                </Button>
              )}
            </div>
            <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label={t("settings.skills.marketplaceCategories")}>
              {MARKETPLACE_CATEGORIES.map((item) => (
                <button
                  key={item}
                  type="button"
                  aria-pressed={category === item}
                  className={cn(
                    "cursor-pointer rounded-full border px-3 py-1.5 text-[11px] font-medium outline-none transition-colors duration-150 focus-visible:ring-2 focus-visible:ring-ring",
                    category === item
                      ? "border-foreground/20 bg-foreground text-background"
                      : "border-border bg-background text-muted-foreground hover:border-foreground/20 hover:text-foreground",
                  )}
                  onClick={() => setCategory(item)}
                >
                  {t(`settings.skills.marketplaceCategory.${item}`)}
                </button>
              ))}
              <span className="ml-auto text-[10px] tabular-nums text-muted-foreground">
                {t("settings.skills.marketplaceResults", { count: visibleSkills.length })}
              </span>
            </div>
          </div>

          {visibleSkills.length === 0 ? (
            <div className="flex flex-col items-center py-14 text-center">
              <span className="flex size-11 items-center justify-center rounded-xl border border-border bg-muted/30 text-muted-foreground">
                <IconSearch className="size-4" />
              </span>
              <p className="mt-3 text-sm font-medium">{t("settings.skills.marketplaceEmpty")}</p>
              <p className="mt-1 text-xs text-muted-foreground">{t("settings.skills.marketplaceEmptyHint")}</p>
            </div>
          ) : (
            <div className="mt-4 grid grid-cols-1 gap-3 sm:grid-cols-2">
              {visibleSkills.map((skill) => {
                const installed = installedIds.has(skill.id);
                const CategoryIcon = MARKETPLACE_CATEGORY_ICONS[skill.category];
                return (
                  <article
                    key={skill.id}
                    className="group relative flex min-h-40 flex-col overflow-hidden rounded-xl border border-border bg-background p-4 transition-[border-color,box-shadow] duration-200 hover:border-foreground/20 hover:shadow-lg hover:shadow-foreground/5"
                  >
                    {skill.featured && (
                      <span className="absolute right-3 top-3 rounded-full border border-primary/20 bg-primary/5 px-2 py-0.5 text-[9px] font-semibold text-primary">
                        {t("settings.skills.marketplaceFeatured")}
                      </span>
                    )}
                    <div className="flex items-center gap-2.5 pr-14">
                      <span className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-border bg-muted/35 text-foreground shadow-sm">
                        <CategoryIcon className="size-4" />
                      </span>
                      <div className="min-w-0">
                        <h3 className="truncate text-sm font-semibold tracking-tight">{skill.name}</h3>
                        <p className="mt-0.5 truncate text-[10px] text-muted-foreground">{skill.publisher}</p>
                      </div>
                    </div>
                    <p className="mt-3 flex-1 text-xs leading-5 text-muted-foreground">{skill.description}</p>
                    <div className="mt-3 flex items-center justify-between gap-3 border-t border-border/70 pt-3">
                      <span className="text-[10px] font-medium text-muted-foreground">
                        {t(`settings.skills.marketplaceCategory.${skill.category}`)}
                      </span>
                      <Button
                        variant={installed ? "ghost" : "secondary"}
                        size="sm"
                        className="min-w-20"
                        disabled={installed}
                        onClick={() => onInstall(skill)}
                      >
                        {installed ? <IconCheck /> : <IconPlus />}
                        {t(installed ? "settings.skills.installed" : "settings.skills.install")}
                      </Button>
                    </div>
                  </article>
                );
              })}
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** Confirms destructive removal before it touches shared state. */
function DeleteAtomDialog({ tPrefix, target, onOpenChange, onDelete }: {
  tPrefix: string;
  target: AtomRecord | null;
  onOpenChange: (open: boolean) => void;
  onDelete: (target: AtomRecord) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const confirm = async () => {
    if (!target || deleting) return;
    setDeleting(true);
    setError(null);
    try {
      await onDelete(target);
      onOpenChange(false);
    } catch {
      setError(t(`${tPrefix}.deleteError`));
    } finally {
      setDeleting(false);
    }
  };

  return (
    <AlertDialog open={target !== null} onOpenChange={(open) => !deleting && onOpenChange(open)}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t(`${tPrefix}.deleteTitle`, { name: target?.name ?? "" })}</AlertDialogTitle>
          <AlertDialogDescription>{t(`${tPrefix}.deleteDescription`)}</AlertDialogDescription>
        </AlertDialogHeader>
        {error && <p className="text-xs text-destructive">{error}</p>}
        <AlertDialogFooter>
          <AlertDialogCancel disabled={deleting}>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction variant="destructive" disabled={deleting} onClick={() => void confirm()}><IconTrash />{deleting ? t("delete.deleting") : t("common.delete")}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}
