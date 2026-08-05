import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { decodeRemoteError, type Agent, type PrepareSkillImportResponse, type Skill, type SkillImportConflictDecision, type SkillImportDecision, type SkillImportSession } from "@ora/contracts";
import { useQueryClient } from "@tanstack/react-query";
import { usePlatform } from "@ora/platform";
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
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Textarea,
  cn,
} from "@ora/ui";
import {
  IconPencil,
  IconPlus,
  IconRobot,
  IconSearch,
  IconSparkles,
  IconTrash,
  IconUpload,
} from "@tabler/icons-react";
import { useContractsClient } from "../../contracts-client-context";
import { localizeContractError } from "../../i18n/contract-error";
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
import { SettingsHeading } from "./settings-heading";
import { queryKeys } from "../../state/hooks/query-keys";

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
  items: AtomRecord[];
  loading: boolean;
  error: boolean;
  onCreate: (name: string, description: string) => Promise<void>;
  onUpdate: (item: AtomRecord, name: string, description: string) => Promise<void>;
  onDelete: (item: AtomRecord) => Promise<void>;
  extraAction?: React.ReactNode;
}

/** The Roles pane manages the configurable agents surfaced to Ora sessions. */
export function RolesSettings() {
  const agentsQuery = useAgents();
  const createAgent = useCreateAgent();
  const updateAgent = useUpdateAgent();
  const deleteAgent = useDeleteAgent();

  return (
    <AtomManager
      tPrefix="settings.roles"
      icon={IconRobot}
      hasBody
      items={agentsQuery.data ?? []}
      loading={agentsQuery.isPending}
      error={agentsQuery.error !== null}
      onCreate={(name, description) => createAgent.mutateAsync({ name, description }).then(() => undefined)}
      onUpdate={(item, name, description) => updateAgent.mutateAsync({ agent: item as Agent, name, description }).then(() => undefined)}
      onDelete={(item) => deleteAgent.mutateAsync({ agentId: item.id }).then(() => undefined)}
    />
  );
}

/** The Skills pane manages the reusable skills surfaced to Ora sessions. */
export function SkillsSettings() {
  const { t } = useTranslation();
  const skillsQuery = useSkills();
  const createSkill = useCreateSkill();
  const updateSkill = useUpdateSkill();
  const deleteSkill = useDeleteSkill();
  const queryClient = useQueryClient();
  const [importOpen, setImportOpen] = useState(false);

  return <>
    <AtomManager
      tPrefix="settings.skills"
      icon={IconSparkles}
      hasBody={false}
      items={skillsQuery.data ?? []}
      loading={skillsQuery.isPending}
      error={skillsQuery.error !== null}
      extraAction={<Button variant="secondary" size="sm" onClick={() => setImportOpen(true)}><IconUpload />{t("settings.skills.import")}</Button>}
      onCreate={(name, description) => createSkill.mutateAsync({ name, description }).then(() => undefined)}
      onUpdate={(item, name, description) => updateSkill.mutateAsync({ skill: item as Skill, name, description }).then(() => undefined)}
      onDelete={(item) => deleteSkill.mutateAsync({ skillId: item.id }).then(() => undefined)}
    />
    <SkillImportDialog
      open={importOpen}
      onOpenChange={setImportOpen}
      onCompleted={() => void queryClient.invalidateQueries({ queryKey: queryKeys.skills })}
    />
  </>;
}

/**
 * The list-and-editor surface shared by both panes. While creating or editing, the toolbar and
 * list are replaced entirely by {@link AtomEditor}; leaving the editor brings the list back.
 */
function AtomManager({ tPrefix, icon, hasBody, items, loading, error, onCreate, onUpdate, onDelete, extraAction }: AtomManagerConfig) {
  const { t } = useTranslation();
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
          validatesSkill={tPrefix === "settings.skills"}
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

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <div className="relative min-w-0 flex-1">
          <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t(`${tPrefix}.search`)} className="pl-8" />
        </div>
        <div className="flex shrink-0 gap-2">
          {extraAction}
          <Button variant="secondary" size="sm" onClick={() => setEditing({ item: null })}>
            <IconPlus />{t(`${tPrefix}.new`)}
          </Button>
        </div>
      </div>

      <div className="overflow-hidden rounded-lg border border-border">
        <div className="flex items-center justify-between border-b border-border bg-muted/40 px-3 py-2 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
          <span>{t(`${tPrefix}.sectionLabel`)}</span>
          <span className="tabular-nums">{items.length}</span>
        </div>
        {loading && <p className="px-4 py-10 text-center text-sm text-muted-foreground">{t(`${tPrefix}.loading`)}</p>}
        {!loading && error && <p className="px-4 py-10 text-center text-sm text-muted-foreground">{t(`${tPrefix}.loadError`)}</p>}
        {!loading && !error && visibleItems.length === 0 && <p className="px-4 py-10 text-center text-sm text-muted-foreground">{t(`${tPrefix}.empty`)}</p>}
        {!loading && !error && visibleItems.map((item) => {
          const Icon = icon;
          return (
            <div key={item.id} className="grid min-h-16 grid-cols-[minmax(0,1fr)_auto] items-center gap-3 border-b border-border px-3 py-2 last:border-b-0 hover:bg-muted/30">
              <div className="flex min-w-0 items-start gap-3">
                <div className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border border-border bg-muted/40 text-muted-foreground">
                  <Icon className="size-4" />
                </div>
                <div className="min-w-0">
                  <p className="truncate text-sm font-medium">{item.name}</p>
                  <p className="mt-0.5 line-clamp-2 text-xs leading-5 text-muted-foreground">{item.description}</p>
                </div>
              </div>
              <div className="flex justify-end gap-1">
                <Button variant="ghost" size="icon-sm" className="text-muted-foreground" aria-label={t("common.edit")} onClick={() => setEditing({ item })}><IconPencil /></Button>
                <Button variant="ghost" size="icon-sm" className="text-destructive hover:bg-destructive/10 hover:text-destructive" aria-label={t("common.delete")} onClick={() => setDeleteTarget(item)}><IconTrash /></Button>
              </div>
            </div>
          );
        })}
      </div>

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
function AtomEditor({ tPrefix, hasBody, validatesSkill, item, onCancel, onSave }: {
  tPrefix: string;
  hasBody: boolean;
  validatesSkill: boolean;
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
  const normalizedName = name.trim();
  const normalizedDescription = description.trim();
  const nameIsValid = !validatesSkill || SKILL_NAME.test(normalizedName);
  const descriptionIsValid = !validatesSkill
    || (normalizedDescription.length > 0 && new TextEncoder().encode(normalizedDescription).length <= 4096);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!normalizedName || !normalizedDescription || !nameIsValid || !descriptionIsValid || saving) return;
    setSaving(true);
    setError(null);
    try {
      await onSave(normalizedName, normalizedDescription);
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
          <Button type="submit" variant="secondary" size="sm" disabled={saving || !normalizedName || !normalizedDescription || !nameIsValid || !descriptionIsValid}>{saving ? t("common.saving") : t("common.save")}</Button>
        </div>
      </div>

      <div className="rounded-xl border border-border bg-muted/20 p-5">
        <div className="divide-y divide-border/60">
          <div className="grid grid-cols-[72px_minmax(0,1fr)] items-center gap-4 pb-3">
            <Label htmlFor="atom-name" className="text-muted-foreground">{t(`${tPrefix}.nameLabel`)}</Label>
            <Input id="atom-name" value={name} onChange={(event) => setName(event.target.value)} placeholder={t(`${tPrefix}.namePlaceholder`)} autoFocus className={INLINE_FIELD} />
          </div>
          {validatesSkill && !nameIsValid && <p className="pb-3 text-xs text-destructive">{t("settings.skills.nameInvalid")}</p>}
          <div className="grid grid-cols-[72px_minmax(0,1fr)] items-start gap-4 pt-3">
            <Label htmlFor="atom-description" className="pt-1.5 text-muted-foreground">{t(`${tPrefix}.descriptionLabel`)}</Label>
            <Textarea id="atom-description" value={description} onChange={(event) => setDescription(event.target.value)} placeholder={t(`${tPrefix}.descriptionPlaceholder`)} className={cn(INLINE_FIELD, "min-h-9 resize-none py-1.5")} />
          </div>
          {validatesSkill && !descriptionIsValid && <p className="pt-2 text-xs text-destructive">{t("settings.skills.descriptionInvalid")}</p>}
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

const SKILL_NAME = /^[A-Za-z0-9._-]+$/;

/** Guides one source through preparation, conflict decisions, and background import progress. */
function SkillImportDialog({ open, onOpenChange, onCompleted }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCompleted: () => void;
}) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const client = useContractsClient();
  const [session, setSession] = useState<SkillImportSession | null>(null);
  const [decisions, setDecisions] = useState<Record<string, SkillImportDecision>>({});
  const [preparing, setPreparing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const folderInput = useRef<HTMLInputElement>(null);
  const archiveInput = useRef<HTMLInputElement>(null);
  const usesBrowserUploads = platform.worktreeStorage.kind === "unsupported";

  const conflictCandidates = session?.candidates.filter((candidate) => candidate.status === "conflict") ?? [];
  const needsDecisions = conflictCandidates.some((candidate) => decisions[candidate.candidateId] === undefined);
  const isCommitting = session?.status === "committing";

  useEffect(() => {
    if (!open || session?.status !== "committing") return undefined;

    const refresh = () => client.skillImport.get({ sessionId: session.sessionId })
      .then((response) => {
        setSession(response.session);
        if (response.session.status === "completed") onCompleted();
      })
      .catch((cause: unknown) => setError(localizeContractError(cause, t)));
    const timer = window.setInterval(refresh, 3_000);
    void refresh();
    return () => window.clearInterval(timer);
  }, [client, onCompleted, open, session?.sessionId, session?.status, t]);

  const close = () => {
    if (isCommitting) return;
    if (session?.status === "prepared") {
      void client.skillImport.cancel({ sessionId: session.sessionId });
    }
    setSession(null);
    setDecisions({});
    setError(null);
    onOpenChange(false);
  };

  const chooseSource = async (kind: "folder" | "archive") => {
    setPreparing(true);
    setError(null);
    try {
      const path = await platform.selectPath({ kind: kind === "folder" ? "directory" : "file" });
      if (path === null) return;
      const response = await client.skillImport.prepare({
        source: kind === "folder" ? { kind, path } : { kind, path, fileName: path },
      });
      setSession(response.session);
    } catch (cause) {
      setError(localizeContractError(cause, t));
    } finally {
      setPreparing(false);
    }
  };

  const prepareBrowserUpload = async (kind: "folder" | "archive", files: FileList | null) => {
    if (files === null || files.length === 0) return;
    setPreparing(true);
    setError(null);
    try {
      const form = new FormData();
      for (const file of Array.from(files)) {
        const sourcePath = kind === "folder" && file.webkitRelativePath ? file.webkitRelativePath : file.name;
        form.append("source", file, sourcePath);
      }
      const response = await fetch(`/api/skill-imports?mode=${kind}`, { method: "POST", body: form });
      const body: unknown = await response.json();
      if (!response.ok) throw decodeRemoteError(body, response.status);
      setSession((body as PrepareSkillImportResponse).session);
    } catch (cause) {
      setError(localizeContractError(cause, t));
    } finally {
      setPreparing(false);
    }
  };

  const commit = async () => {
    if (session === null || needsDecisions) return;
    const frozenDecisions: Array<SkillImportConflictDecision> = conflictCandidates.map((candidate) => ({
      candidateId: candidate.candidateId,
      decision: decisions[candidate.candidateId],
    }));
    setError(null);
    try {
      const response = await client.skillImport.commit({ sessionId: session.sessionId, decisions: frozenDecisions });
      setSession((current) => current === null ? current : { ...current, status: response.status, progress: response.progress });
    } catch (cause) {
      setError(localizeContractError(cause, t));
    }
  };

  const reset = () => {
    setSession(null);
    setDecisions({});
    setError(null);
  };

  return <Dialog open={open} onOpenChange={(nextOpen) => nextOpen || close()}>
    <DialogContent className="max-w-xl">
      <DialogHeader>
        <DialogTitle>{t("settings.skills.importTitle")}</DialogTitle>
        <DialogDescription>{t("settings.skills.importDescription")}</DialogDescription>
      </DialogHeader>
      {session === null && <div className="grid gap-3 sm:grid-cols-2">
        <input {...{ webkitdirectory: "" }} ref={folderInput} className="hidden" type="file" multiple onChange={(event) => { void prepareBrowserUpload("folder", event.currentTarget.files); event.currentTarget.value = ""; }} />
        <input ref={archiveInput} className="hidden" type="file" accept=".zip,.skill,.tar.gz,.tgz" onChange={(event) => { void prepareBrowserUpload("archive", event.currentTarget.files); event.currentTarget.value = ""; }} />
        <Button variant="secondary" disabled={preparing} onClick={() => usesBrowserUploads ? folderInput.current?.click() : void chooseSource("folder")}>{t("settings.skills.importFolder")}</Button>
        <Button variant="secondary" disabled={preparing} onClick={() => usesBrowserUploads ? archiveInput.current?.click() : void chooseSource("archive")}>{t("settings.skills.importArchive")}</Button>
      </div>}
      {session !== null && <div className="space-y-3">
        <p className="text-sm text-muted-foreground">{t("settings.skills.importProgress", { processed: session.progress.processed, total: session.progress.total })}</p>
        <div className="max-h-72 space-y-2 overflow-y-auto rounded-md border p-3">
          {session.candidates.map((candidate) => <div key={candidate.candidateId} className="space-y-1 border-b pb-2 last:border-0 last:pb-0">
            <div className="flex items-center justify-between gap-3"><span className="font-medium">{candidate.name || candidate.sourcePath}</span><span className="text-xs text-muted-foreground">{candidate.status}</span></div>
            <p className="text-xs text-muted-foreground">{candidate.sourcePath} · {candidate.fileCount} {t("settings.skills.importFiles")}</p>
            {candidate.status === "invalid" && <p className="text-xs text-destructive">{candidate.errorCode}</p>}
            {candidate.status === "conflict" && session.status === "prepared" && <div className="flex items-center gap-2 text-xs">
              <span>{t("settings.skills.importExisting", { description: candidate.existingSkill?.description })}</span>
              <Button size="sm" variant={decisions[candidate.candidateId] === "skip" ? "secondary" : "ghost"} onClick={() => setDecisions((current) => ({ ...current, [candidate.candidateId]: "skip" }))}>{t("settings.skills.importSkip")}</Button>
              <Button size="sm" variant={decisions[candidate.candidateId] === "overwrite" ? "secondary" : "ghost"} onClick={() => setDecisions((current) => ({ ...current, [candidate.candidateId]: "overwrite" }))}>{t("settings.skills.importOverwrite")}</Button>
            </div>}
          </div>)}
          {session.progress.results.map((result) => <div key={result.candidateId} className="flex items-center justify-between gap-3 text-sm"><span>{result.name}</span><span className="text-muted-foreground">{result.status}</span></div>)}
        </div>
        {session.status === "completed" && <p className="text-sm text-muted-foreground">{t("settings.skills.importCompleted")}</p>}
      </div>}
      {error && <p className="text-sm text-destructive">{error}</p>}
      <DialogFooter>
        {session?.status === "completed" && <Button variant="secondary" onClick={reset}>{t("settings.skills.importAnother")}</Button>}
        <Button variant="ghost" disabled={isCommitting} onClick={close}>{t("common.cancel")}</Button>
        {session?.status === "prepared" && <Button disabled={needsDecisions} onClick={() => void commit()}>{t("settings.skills.importCommit")}</Button>}
      </DialogFooter>
    </DialogContent>
  </Dialog>;
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
