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
  Badge,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Tabs,
  TabsList,
  TabsTrigger,
  Textarea,
} from "@ora/ui";
import {
  IconPencil,
  IconPlus,
  IconRobot,
  IconSearch,
  IconSparkles,
  IconTrash,
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
import { SettingsHeading } from "./settings-heading";

type AtomKind = "agent" | "skill";
type AtomRecord = Agent | Skill;

interface EditorState {
  kind: AtomKind;
  item: AtomRecord | null;
}

interface DeleteState {
  kind: AtomKind;
  item: AtomRecord;
}

/** Manages configurable agents and skills through react-query backed mutations. */
export function AtomsSettings() {
  const { t } = useTranslation();
  const agentsQuery = useAgents();
  const skillsQuery = useSkills();
  const agents = agentsQuery.data ?? [];
  const skills = skillsQuery.data ?? [];
  const loading = agentsQuery.isPending || skillsQuery.isPending;
  const error = agentsQuery.error ?? skillsQuery.error;

  const createAgent = useCreateAgent();
  const updateAgent = useUpdateAgent();
  const deleteAgent = useDeleteAgent();
  const createSkill = useCreateSkill();
  const updateSkill = useUpdateSkill();
  const deleteSkill = useDeleteSkill();

  const [kind, setKind] = useState<AtomKind>("agent");
  const [query, setQuery] = useState("");
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<DeleteState | null>(null);

  const source = kind === "agent" ? agents : skills;
  const needle = query.trim().toLowerCase();
  const visibleItems = useMemo(
    () => source.filter((item) => !needle
      || item.name.toLowerCase().includes(needle)
      || item.description.toLowerCase().includes(needle)),
    [needle, source],
  );

  const saveItem = async (editorKind: AtomKind, item: AtomRecord | null, name: string, description: string) => {
    if (editorKind === "agent") {
      if (item) await updateAgent.mutateAsync({ agent: item, name, description });
      else await createAgent.mutateAsync({ name, description });
      return;
    }
    if (item) await updateSkill.mutateAsync({ skill: item, name, description });
    else await createSkill.mutateAsync({ name, description });
  };

  const deleteItem = async (target: DeleteState) => {
    if (target.kind === "agent") await deleteAgent.mutateAsync({ agentId: target.item.id });
    else await deleteSkill.mutateAsync({ skillId: target.item.id });
  };

  return (
    <div className="space-y-5">
      <SettingsHeading title={t("settings.atoms.title")} description={t("settings.atoms.description")} />

      <Tabs value={kind} onValueChange={(value) => setKind(value as AtomKind)}>
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center">
          <TabsList className="shrink-0">
            <TabsTrigger value="agent"><IconRobot />{t("settings.atoms.agents")}<Badge variant="secondary" className="ml-1 h-4 px-1.5 text-[10px]">{agents.length}</Badge></TabsTrigger>
            <TabsTrigger value="skill"><IconSparkles />{t("settings.atoms.skills")}<Badge variant="secondary" className="ml-1 h-4 px-1.5 text-[10px]">{skills.length}</Badge></TabsTrigger>
          </TabsList>
          <div className="relative min-w-0 flex-1">
            <IconSearch className="absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
            <Input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("settings.atoms.search")} className="pl-8" />
          </div>
          <Button onClick={() => setEditor({ kind, item: null })}><IconPlus />{kind === "agent" ? t("settings.atoms.newAgent") : t("settings.atoms.newSkill")}</Button>
        </div>
      </Tabs>

      <div className="overflow-hidden rounded-lg border border-border">
        <div className="grid grid-cols-[minmax(0,1fr)_88px] border-b border-border bg-muted/40 px-3 py-2 text-[11px] font-semibold uppercase text-muted-foreground">
          <span>{kind === "agent" ? t("settings.atoms.agents") : t("settings.atoms.skills")}</span>
          <span className="text-right">{t("settings.atoms.commands")}</span>
        </div>
        {loading && <p className="px-4 py-10 text-center text-sm text-muted-foreground">{t("settings.atoms.loading")}</p>}
        {!loading && error && (
          <div className="flex flex-col items-center gap-3 px-4 py-10 text-center">
            <p className="text-sm text-destructive">{t("settings.atoms.loadError")}</p>
          </div>
        )}
        {!loading && !error && visibleItems.length === 0 && <p className="px-4 py-10 text-center text-sm text-muted-foreground">{t("settings.atoms.empty")}</p>}
        {!loading && !error && visibleItems.map((item) => (
          <div key={item.id} className="grid min-h-16 grid-cols-[minmax(0,1fr)_88px] items-center gap-3 border-b border-border px-3 py-2 last:border-b-0 hover:bg-muted/30">
            <div className="flex min-w-0 items-start gap-3">
              <div className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-md border border-border bg-background">
                {kind === "agent" ? <IconRobot className="size-4 text-emerald-600" /> : <IconSparkles className="size-4 text-amber-600" />}
              </div>
              <div className="min-w-0">
                <p className="truncate text-sm font-medium">{item.name}</p>
                <p className="mt-0.5 line-clamp-2 text-xs leading-5 text-muted-foreground">{item.description}</p>
              </div>
            </div>
            <div className="flex justify-end gap-1">
              <Button variant="ghost" size="icon-sm" aria-label={t("common.edit")} onClick={() => setEditor({ kind, item })}><IconPencil /></Button>
              <Button variant="ghost" size="icon-sm" className="text-destructive hover:text-destructive" aria-label={t("common.delete")} onClick={() => setDeleteTarget({ kind, item })}><IconTrash /></Button>
            </div>
          </div>
        ))}
      </div>

      <AtomEditorDialog key={`${editor?.kind}-${editor?.item?.id ?? "new"}`} editor={editor} onOpenChange={(open) => !open && setEditor(null)} onSave={saveItem} />
      <DeleteAtomDialog target={deleteTarget} onOpenChange={(open) => !open && setDeleteTarget(null)} onDelete={deleteItem} />
    </div>
  );
}

/** Provides a compact form for both agent and skill mutations because their current contracts share one shape. */
function AtomEditorDialog({ editor, onOpenChange, onSave }: { editor: EditorState | null; onOpenChange: (open: boolean) => void; onSave: (kind: AtomKind, item: AtomRecord | null, name: string, description: string) => Promise<void> }) {
  const { t } = useTranslation();
  // Lazy-init from editor; caller passes a `key` to remount when the editor target changes.
  const [name, setName] = useState(() => editor?.item?.name ?? "");
  const [description, setDescription] = useState(() => editor?.item?.description ?? "");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (!editor || !name.trim() || !description.trim() || saving) return;
    setSaving(true);
    setError(null);
    try {
      await onSave(editor.kind, editor.item, name.trim(), description.trim());
      onOpenChange(false);
    } catch {
      setError(t("settings.atoms.saveError"));
    } finally {
      setSaving(false);
    }
  };

  const entityLabel = editor?.kind === "skill" ? t("settings.atoms.skill") : t("settings.atoms.agent");
  return (
    <Dialog open={editor !== null} onOpenChange={(open) => !saving && onOpenChange(open)}>
      <DialogContent>
        <form onSubmit={(event) => void submit(event)} className="space-y-4">
          <DialogHeader>
            <DialogTitle>{editor?.item ? t("settings.atoms.editTitle", { type: entityLabel }) : t("settings.atoms.createTitle", { type: entityLabel })}</DialogTitle>
            <DialogDescription>{t("settings.atoms.formDescription", { type: entityLabel })}</DialogDescription>
          </DialogHeader>
          <div className="space-y-1.5">
            <Label htmlFor="atom-name">{t("settings.atoms.name")}</Label>
            <Input id="atom-name" value={name} onChange={(event) => setName(event.target.value)} autoFocus />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="atom-description">{t("settings.atoms.descriptionLabel")}</Label>
            <Textarea id="atom-description" value={description} onChange={(event) => setDescription(event.target.value)} className="min-h-24 resize-none" />
          </div>
          {error && <p className="text-xs text-destructive">{error}</p>}
          <DialogFooter>
            <Button type="button" variant="outline" disabled={saving} onClick={() => onOpenChange(false)}>{t("common.cancel")}</Button>
            <Button type="submit" disabled={saving || !name.trim() || !description.trim()}>{saving ? t("common.saving") : t("settings.atoms.save")}</Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

/** Confirms destructive Atom commands before changing shared mock state. */
function DeleteAtomDialog({ target, onOpenChange, onDelete }: { target: DeleteState | null; onOpenChange: (open: boolean) => void; onDelete: (target: DeleteState) => Promise<void> }) {
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
      setError(t("settings.atoms.deleteError"));
    } finally {
      setDeleting(false);
    }
  };

  return (
    <AlertDialog open={target !== null} onOpenChange={(open) => !deleting && onOpenChange(open)}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("settings.atoms.deleteTitle", { name: target?.item.name ?? "" })}</AlertDialogTitle>
          <AlertDialogDescription>{t("settings.atoms.deleteDescription")}</AlertDialogDescription>
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
