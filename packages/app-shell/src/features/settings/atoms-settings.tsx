import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { decodeRemoteError, type Agent, type AgentImportCandidate, type AgentImportDecision, type PrepareSkillImportResponse, type Skill, type SkillImportConflictDecision, type SkillImportDecision, type SkillImportSession } from "@ora/contracts";
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
import { SkillMarketplacePanel } from "./skill-marketplace-panel";

type AtomRecord = Agent | Skill;
type TablerIcon = typeof IconRobot;

/** The i18n namespace and behaviour that distinguish the two atom panes. */
interface AtomManagerConfig {
  /** Translation key prefix, e.g. `settings.roles`. */
  tPrefix: string;
  /** Neutral mark drawn beside each row. */
  icon: TablerIcon;
  /** Loads persisted Markdown only while an existing item is open for editing. */
  loadContent: (item: AtomRecord) => Promise<string>;
  items: AtomRecord[];
  loading: boolean;
  error: boolean;
  onCreate: (name: string, description: string, content: string) => Promise<void>;
  onUpdate: (item: AtomRecord, name: string, description: string, content: string) => Promise<void>;
  onDelete: (item: AtomRecord) => Promise<void>;
  extraAction?: ReactNode;
  /** Optional host-specific surface shown between the pane heading and local atom controls. */
  intro?: ReactNode;
  /** Chooses between compact rows and the wider card grid used by Skills. */
  presentation?: "list" | "grid";
}

/** The Roles pane manages the configurable agents surfaced to Ora sessions. */
export function RolesSettings() {
  const { t } = useTranslation();
  const client = useContractsClient();
  const agentsQuery = useAgents();
  const createAgent = useCreateAgent();
  const updateAgent = useUpdateAgent();
  const deleteAgent = useDeleteAgent();
  const queryClient = useQueryClient();
  const [importOpen, setImportOpen] = useState(false);

  return <>
    <AtomManager
      tPrefix="settings.roles"
      icon={IconRobot}
      loadContent={(item) => client.agent.get({ agentId: item.id }).then((response) => response.agent.content)}
      items={agentsQuery.data ?? []}
      loading={agentsQuery.isPending}
      error={agentsQuery.error !== null}
      extraAction={<Button variant="outline" size="sm" onClick={() => setImportOpen(true)}><IconUpload />{t("settings.roles.import")}</Button>}
      onCreate={(name, description, content) => createAgent.mutateAsync({ name, description, content }).then(() => undefined)}
      onUpdate={(item, name, description, content) => updateAgent.mutateAsync({ agent: item as Agent, name, description, content }).then(() => undefined)}
      onDelete={(item) => deleteAgent.mutateAsync({ agentId: item.id }).then(() => undefined)}
    />
    <AgentImportDialog
      open={importOpen}
      onOpenChange={setImportOpen}
      onCompleted={() => void queryClient.invalidateQueries({ queryKey: queryKeys.agents })}
    />
  </>;
}

/** The Skills pane manages the reusable skills surfaced to Ora sessions. */
export function SkillsSettings() {
  const { t } = useTranslation();
  const skillsQuery = useSkills();
  const createSkill = useCreateSkill();
  const updateSkill = useUpdateSkill();
  const deleteSkill = useDeleteSkill();
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const [importOpen, setImportOpen] = useState(false);

  return <>
    <AtomManager
      tPrefix="settings.skills"
      icon={IconSparkles}
      loadContent={(item) => client.skill.get({ skillId: item.id }).then((response) => response.skill.content)}
      items={skillsQuery.data ?? []}
      loading={skillsQuery.isPending}
      error={skillsQuery.error !== null}
      extraAction={<Button variant="outline" size="sm" onClick={() => setImportOpen(true)}><IconUpload />{t("settings.skills.import")}</Button>}
      intro={<SkillMarketplacePanel />}
      presentation="grid"
      onCreate={(name, description, content) => createSkill.mutateAsync({ name, description, content }).then(() => undefined)}
      onUpdate={(item, name, description, content) => updateSkill.mutateAsync({ skill: item as Skill, name, description, content }).then(() => undefined)}
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
function AtomManager({ tPrefix, icon, loadContent, items, loading, error, onCreate, onUpdate, onDelete, extraAction, intro, presentation = "list" }: AtomManagerConfig) {
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

  const save = async (name: string, description: string, content: string) => {
    if (editing?.item) await onUpdate(editing.item, name, description, content);
    else await onCreate(name, description, content);
    setEditing(null);
  };

  if (editing !== null) {
    return (
      <div className="space-y-5">
        <SettingsHeading title={t(`${tPrefix}.title`)} description={t(`${tPrefix}.description`)} />
        <AtomEditor
          key={editing.item?.id ?? "new"}
          tPrefix={tPrefix}
          loadContent={loadContent}
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

      {intro}

      <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
        <div className="relative min-w-0 flex-1">
          <IconSearch className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t(`${tPrefix}.search`)} className="pl-8" />
        </div>
        <div className="flex shrink-0 gap-2">
          {extraAction}
          <Button variant="outline" size="sm" onClick={() => setEditing({ item: null })}>
            <IconPlus />{t(`${tPrefix}.new`)}
          </Button>
        </div>
      </div>

      <div className={cn(presentation === "list" && "overflow-hidden rounded-lg border border-border")}>
        <div className={cn(
          "flex items-center justify-between text-[11px] font-semibold uppercase tracking-wide text-muted-foreground",
          presentation === "list"
            ? "border-b border-border bg-muted/40 px-3 py-2"
            : "px-1 py-1",
        )}>
          <span>{t(`${tPrefix}.sectionLabel`)}</span>
          <span className="tabular-nums">{items.length}</span>
        </div>
        <div
          role="list"
          aria-label={t(`${tPrefix}.sectionLabel`)}
          className={cn(presentation === "grid" && "mt-2 grid gap-3 md:grid-cols-2")}
        >
          {loading && <p className={cn("px-4 py-10 text-center text-sm text-muted-foreground", presentation === "grid" && "rounded-lg border border-border md:col-span-2")}>{t(`${tPrefix}.loading`)}</p>}
          {!loading && error && <p className={cn("px-4 py-10 text-center text-sm text-muted-foreground", presentation === "grid" && "rounded-lg border border-border md:col-span-2")}>{t(`${tPrefix}.loadError`)}</p>}
          {!loading && !error && visibleItems.length === 0 && <p className={cn("px-4 py-10 text-center text-sm text-muted-foreground", presentation === "grid" && "rounded-lg border border-border md:col-span-2")}>{t(`${tPrefix}.empty`)}</p>}
          {!loading && !error && visibleItems.map((item) => {
            const Icon = icon;
            return (
              <div
                key={item.id}
                role="listitem"
                className={cn(
                  "grid grid-cols-[minmax(0,1fr)_auto] gap-3 px-3 py-2 hover:bg-muted/30",
                  presentation === "list"
                    ? "min-h-16 items-center border-b border-border last:border-b-0"
                    : "min-h-28 items-start rounded-lg border border-border bg-background p-3",
                )}
              >
                <div className="flex min-w-0 items-start gap-3">
                  <div className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border border-border bg-muted/40 text-muted-foreground">
                    <Icon className="size-4" />
                  </div>
                  <div className="min-w-0">
                    <p className="truncate text-sm font-medium">{item.name}</p>
                    <p className={cn("mt-0.5 text-xs leading-5 text-muted-foreground", presentation === "grid" ? "line-clamp-3" : "line-clamp-2")}>{item.description}</p>
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
      </div>

      <DeleteAtomDialog tPrefix={tPrefix} target={deleteTarget} onOpenChange={(open) => !open && setDeleteTarget(null)} onDelete={onDelete} />
    </div>
  );
}

/** Borderless field styling so name and description read as inline text inside the card. */
const INLINE_FIELD = "border-transparent bg-transparent px-0 shadow-none focus-visible:border-transparent focus-visible:ring-0 dark:bg-transparent";

/** The full-surface create/edit form for metadata and Markdown body. */
function AtomEditor({ tPrefix, loadContent, validatesSkill, item, onCancel, onSave }: {
  tPrefix: string;
  loadContent: (item: AtomRecord) => Promise<string>;
  validatesSkill: boolean;
  item: AtomRecord | null;
  onCancel: () => void;
  onSave: (name: string, description: string, content: string) => Promise<void>;
}) {
  const { t } = useTranslation();
  const [name, setName] = useState(() => item?.name ?? "");
  const [description, setDescription] = useState(() => item?.description ?? "");
  const [content, setContent] = useState<string | null>(() => item ? null : "");
  const [contentError, setContentError] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (item === null) return undefined;
    let active = true;
    void loadContent(item)
      .then((nextContent) => {
        if (active) setContent(nextContent);
      })
      .catch(() => {
        if (active) setContentError(true);
      });
    return () => {
      active = false;
    };
  }, [item, loadContent]);

  const normalizedName = name.trim();
  const normalizedDescription = description.trim();
  const nameIsValid = !validatesSkill || SKILL_NAME.test(normalizedName);
  const descriptionIsValid = !validatesSkill
    || (normalizedDescription.length > 0 && new TextEncoder().encode(normalizedDescription).length <= 4096);
  const contentReady = content !== null && !contentError;

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!normalizedName || !normalizedDescription || !nameIsValid || !descriptionIsValid || !contentReady || saving) return;
    setSaving(true);
    setError(null);
    try {
      await onSave(normalizedName, normalizedDescription, content);
    } catch (cause) {
      setError(localizeContractError(cause, t));
      setSaving(false);
    }
  };

  return (
    <form onSubmit={(event) => void submit(event)} className="space-y-5">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-medium">{item ? t(`${tPrefix}.editTitle`) : t(`${tPrefix}.createTitle`)}</h3>
        <div className="flex items-center gap-2">
          <Button type="button" variant="ghost" size="sm" disabled={saving} onClick={onCancel}>{t("common.cancel")}</Button>
          <Button type="submit" variant="secondary" size="sm" disabled={saving || !normalizedName || !normalizedDescription || !nameIsValid || !descriptionIsValid || !contentReady}>{saving ? t("common.saving") : t("common.save")}</Button>
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

      <div className="space-y-1.5">
        <Label htmlFor="atom-content" className="px-1 text-muted-foreground">{t(`${tPrefix}.contentLabel`)}</Label>
        <Textarea
          id="atom-content"
          value={content ?? ""}
          onChange={(event) => setContent(event.target.value)}
          disabled={!contentReady}
          placeholder={content === null ? t(`${tPrefix}.contentLoading`) : undefined}
          className="min-h-56 resize-y font-mono text-sm"
        />
        <p className="px-1 text-[11px] leading-4 text-muted-foreground">{t(`${tPrefix}.contentHint`)}</p>
        {contentError && <p className="px-1 text-xs text-destructive">{t(`${tPrefix}.contentLoadError`)}</p>}
      </div>

      {error && <p className="text-xs text-destructive">{error}</p>}
    </form>
  );
}
interface AgentImportPreview {
  fileName: string;
  content: string;
  candidate: AgentImportCandidate;
}

/** Imports one local Agent Markdown file through preview and frozen conflict decisions. */
function AgentImportDialog({ open, onOpenChange, onCompleted }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCompleted: () => void;
}) {
  const { t } = useTranslation();
  const client = useContractsClient();
  const input = useRef<HTMLInputElement>(null);
  const [preview, setPreview] = useState<AgentImportPreview | null>(null);
  const [decision, setDecision] = useState<AgentImportDecision | null>(null);
  const [preparing, setPreparing] = useState(false);
  const [committing, setCommitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reset = () => {
    setPreview(null);
    setDecision(null);
    setError(null);
  };

  const close = () => {
    if (preparing || committing) return;
    reset();
    onOpenChange(false);
  };

  const prepare = async (file: File) => {
    if (!file.name.toLowerCase().endsWith(".md")) {
      setError(t("settings.roles.importInvalidFile"));
      return;
    }
    setPreparing(true);
    setError(null);
    try {
      const content = await file.text();
      const response = await client.agentImport.prepare({ content });
      setPreview({ fileName: file.name, content, candidate: response.candidate });
      setDecision(null);
    } catch (cause) {
      setError(localizeContractError(cause, t));
    } finally {
      setPreparing(false);
    }
  };

  const commit = async () => {
    if (preview === null || (preview.candidate.status === "conflict" && decision === null)) return;
    setCommitting(true);
    setError(null);
    try {
      const existing = preview.candidate.existingAgent;
      const response = await client.agentImport.commit({
        content: preview.content,
        decision: preview.candidate.status === "conflict" ? decision : null,
        expectedAgentId: existing?.agentId ?? null,
        expectedUpdatedAt: existing?.updatedAt ?? null,
      });
      if (response.status === "stale_conflict") {
        const refreshed = await client.agentImport.prepare({ content: preview.content });
        setPreview((current) => current === null ? current : { ...current, candidate: refreshed.candidate });
        setDecision(null);
        setError(t("settings.roles.importStale"));
        return;
      }
      onCompleted();
      reset();
      onOpenChange(false);
    } catch (cause) {
      setError(localizeContractError(cause, t));
    } finally {
      setCommitting(false);
    }
  };

  const conflict = preview?.candidate.status === "conflict";
  const canCommit = preview !== null && (!conflict || decision !== null) && !committing;

  return <Dialog open={open} onOpenChange={(nextOpen) => nextOpen || close()}>
    <DialogContent className="max-w-lg">
      <DialogHeader>
        <DialogTitle>{t("settings.roles.importTitle")}</DialogTitle>
        <DialogDescription>{t("settings.roles.importDescription")}</DialogDescription>
      </DialogHeader>
      <input
        ref={input}
        className="hidden"
        type="file"
        accept=".md,text/markdown"
        onChange={(event) => {
          const file = event.currentTarget.files?.item(0);
          if (file) void prepare(file);
          event.currentTarget.value = "";
        }}
      />
      {preview === null ? (
        <Button variant="outline" disabled={preparing} onClick={() => input.current?.click()}>
          <IconUpload />{preparing ? t("settings.roles.importPreparing") : t("settings.roles.importChoose")}
        </Button>
      ) : (
        <div className="space-y-3 rounded-lg border border-border p-4">
          <div>
            <p className="text-sm font-medium">{preview.candidate.name}</p>
            <p className="text-xs text-muted-foreground">{preview.fileName}</p>
          </div>
          <p className="text-sm text-muted-foreground">{preview.candidate.description}</p>
          {conflict && (
            <div className="space-y-2">
              <p className="text-xs text-muted-foreground">
                {t("settings.roles.importExisting", { description: preview.candidate.existingAgent?.description })}
              </p>
              <div className="flex gap-2">
                <Button size="sm" variant={decision === "skip" ? "secondary" : "outline"} onClick={() => setDecision("skip")}>
                  {t("settings.roles.importSkip")}
                </Button>
                <Button size="sm" variant={decision === "overwrite" ? "secondary" : "outline"} onClick={() => setDecision("overwrite")}>
                  {t("settings.roles.importOverwrite")}
                </Button>
              </div>
            </div>
          )}
        </div>
      )}
      {error && <p className="text-sm text-destructive">{error}</p>}
      <DialogFooter>
        {preview !== null && <Button variant="outline" disabled={committing} onClick={reset}>{t("settings.roles.importChooseAnother")}</Button>}
        <Button variant="ghost" disabled={preparing || committing} onClick={close}>{t("common.cancel")}</Button>
        {preview !== null && <Button disabled={!canCommit} onClick={() => void commit()}>{committing ? t("settings.roles.importCommitting") : t("settings.roles.importCommit")}</Button>}
      </DialogFooter>
    </DialogContent>
  </Dialog>;
}

const SKILL_NAME = /^[A-Za-z0-9._-]+$/;

/** Guides one source through preparation, conflict decisions, and background import progress. */
export function SkillImportDialog({ open, onOpenChange, onCompleted, initialSession = null }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onCompleted: () => void;
  initialSession?: SkillImportSession | null;
}) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const client = useContractsClient();
  const [session, setSession] = useState<SkillImportSession | null>(() => initialSession);
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
