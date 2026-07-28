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
  Input,
  Label,
  Textarea,
  cn,
} from "@ora/ui";
import {
  IconCloudOff,
  IconPencil,
  IconPlus,
  IconRobot,
  IconSearch,
  IconSparkles,
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
  MOCK_AGENTS,
  MOCK_SKILLS,
  isHuaweiAtom,
  isMockAtom,
} from "./atom-mock-catalog";
import { SettingsHeading } from "./settings-heading";

type AtomRecord = Agent | Skill;
type TablerIcon = typeof IconRobot;

/** The i18n namespace and behaviour that distinguish the two atom panes. */
interface AtomManagerConfig {
  /** Translation key prefix, e.g. `settings.roles`. */
  tPrefix: string;
  /** Neutral mark drawn beside each row. */
  icon: TablerIcon;
  /** Roles carry an extra, prototype-only body field; skills do not. */
  hasBody: boolean;
  /** Restrained accent used to distinguish role and skill catalogs at a glance. */
  accentClassName: string;
  items: AtomRecord[];
  remoteError: boolean;
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
  const [mockAgents, setMockAgents] = useState<Agent[]>(() => [...MOCK_AGENTS]);

  return (
    <AtomManager
      tPrefix="settings.roles"
      icon={IconRobot}
      hasBody
      accentClassName="border-sky-500/20 bg-sky-500/10 text-sky-700 dark:text-sky-300"
      items={[...(agentsQuery.data ?? []), ...mockAgents]}
      remoteError={agentsQuery.error !== null}
      onCreate={(name, description) => createAgent.mutateAsync({ name, description }).then(() => undefined)}
      onUpdate={(item, name, description) => {
        if (isMockAtom(item)) {
          setMockAgents((current) => current.map((agent) => (
            agent.id === item.id ? { ...agent, name, description } : agent
          )));
          return Promise.resolve();
        }
        return updateAgent.mutateAsync({ agent: item as Agent, name, description }).then(() => undefined);
      }}
      onDelete={(item) => {
        if (isMockAtom(item)) {
          setMockAgents((current) => current.filter((agent) => agent.id !== item.id));
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
  const [mockSkills, setMockSkills] = useState<Skill[]>(() => [...MOCK_SKILLS]);

  return (
    <AtomManager
      tPrefix="settings.skills"
      icon={IconSparkles}
      hasBody={false}
      accentClassName="border-amber-500/20 bg-amber-500/10 text-amber-700 dark:text-amber-300"
      items={[...(skillsQuery.data ?? []), ...mockSkills]}
      remoteError={skillsQuery.error !== null}
      onCreate={(name, description) => createSkill.mutateAsync({ name, description }).then(() => undefined)}
      onUpdate={(item, name, description) => {
        if (isMockAtom(item)) {
          setMockSkills((current) => current.map((skill) => (
            skill.id === item.id ? { ...skill, name, description } : skill
          )));
          return Promise.resolve();
        }
        return updateSkill.mutateAsync({ skill: item as Skill, name, description }).then(() => undefined);
      }}
      onDelete={(item) => {
        if (isMockAtom(item)) {
          setMockSkills((current) => current.filter((skill) => skill.id !== item.id));
          return Promise.resolve();
        }
        return deleteSkill.mutateAsync({ skillId: item.id }).then(() => undefined);
      }}
    />
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
  remoteError,
  onCreate,
  onUpdate,
  onDelete,
}: AtomManagerConfig) {
  const { t } = useTranslation();
  const CatalogIcon = icon;
  const [query, setQuery] = useState("");
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
              onChange={(event) => setQuery(event.target.value)}
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
                onClick={() => setQuery("")}
              >
                <IconX />
              </Button>
            )}
          </div>
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
            <Button variant="ghost" size="sm" className="mt-3" onClick={() => setQuery("")}>{t(`${tPrefix}.clearSearch`)}</Button>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-2.5 p-3 md:grid-cols-2">
            {visibleItems.map((item) => {
              const Icon = icon;
              const mock = isMockAtom(item);
              const huawei = isHuaweiAtom(item);
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
                      {mock && (
                        <span className="rounded-full border border-border bg-muted/50 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide text-muted-foreground">
                          {t(`${tPrefix}.mockBadge`)}
                        </span>
                      )}
                      {huawei && (
                        <span className="rounded-full border border-red-500/20 bg-red-500/5 px-1.5 py-0.5 text-[9px] font-semibold text-red-600 dark:text-red-400">
                          {t(`${tPrefix}.huaweiBadge`)}
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
 * are prototype-only affordances that are intentionally not wired to the backend yet.
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
