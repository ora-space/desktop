import { useState } from "react";
import type {
  SpecSource,
  SpecSourceVisibility,
  SpecTarget,
  SpecWorkflow,
} from "@ora/contracts";
import { usePlatform } from "@ora/platform";
import {
  Button,
  Badge,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Switch,
} from "@ora/ui";
import { IconFolderPlus, IconTrash } from "@tabler/icons-react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { useContractsClient } from "../../contracts-client-context";
import { localizeContractError } from "../../i18n/contract-error";
import { queryKeys } from "../../state/hooks/query-keys";

/** Maps stored workflow kinds to the labels shown in the source editor. */
function workflowKindLabel(kind: string, customName: string, t: (key: string) => string): string {
  if (kind === "open_spec") return "OpenSpec";
  if (kind === "superpowers") return "Superpowers";
  if (kind === "custom") return customName || t("specs.custom");
  return kind;
}

interface SpecSourceDialogProps {
  open: boolean;
  projectId: string;
  target: SpecTarget;
  initialPath: string | undefined;
  sources: SpecSource[];
  onOpenChange: (open: boolean) => void;
}

/** Edits project-wide source overrides without coupling the form to workflow session state. */
export function SpecSourceDialog({
  open,
  projectId,
  target,
  initialPath,
  sources,
  onOpenChange,
}: SpecSourceDialogProps) {
  if (!open) return null;

  return (
    <OpenSpecSourceDialog
      projectId={projectId}
      target={target}
      initialPath={initialPath}
      sources={sources}
      onOpenChange={onOpenChange}
    />
  );
}

/** Owns one editing session so closing the dialog discards its transient draft. */
function OpenSpecSourceDialog({
  projectId,
  target,
  initialPath,
  sources,
  onOpenChange,
}: Omit<SpecSourceDialogProps, "open">) {
  const { t } = useTranslation();
  const platform = usePlatform();
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const [rows, setRows] = useState<SpecSource[]>(sources);
  const [busy, setBusy] = useState<"selecting" | "saving" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const addDirectory = async () => {
    setBusy("selecting");
    setError(null);
    try {
      const absolutePath = await platform.selectPath({ kind: "directory", initialPath });
      if (absolutePath === null) return;
      const resolved = await client.spec.resolveSource({ target, absolutePath });
      setRows((current) => {
        if (current.some((source) => source.relativePath === resolved.relativePath)) {
          return current;
        }
        return [...current, {
          relativePath: resolved.relativePath,
          workflow: resolved.workflow,
          origin: "manual",
          visibility: "enabled",
          availability: "available",
        }];
      });
    } catch (cause) {
      setError(localizeContractError(cause, t));
    } finally {
      setBusy(null);
    }
  };

  const save = async () => {
    setBusy("saving");
    setError(null);
    try {
      await client.spec.updateProjectSources({
        projectId,
        sources: rows.map(({ relativePath, workflow, visibility }) => ({
          relativePath,
          workflow,
          visibility,
        })),
      });
      await queryClient.invalidateQueries({ queryKey: queryKeys.specs(projectId) });
      onOpenChange(false);
    } catch (cause) {
      setError(localizeContractError(cause, t));
    } finally {
      setBusy(null);
    }
  };

  return (
    <Dialog open onOpenChange={(next) => (busy === null || next) && onOpenChange(next)}>
      <DialogContent className="max-w-3xl sm:max-w-3xl">
        <DialogHeader className="gap-2">
          <DialogTitle>{t("specs.sourcesTitle")}</DialogTitle>
          <DialogDescription>{t("specs.sourcesDescription")}</DialogDescription>
        </DialogHeader>
        {initialPath !== undefined && (
          <div className="min-w-0 rounded-md bg-muted/40 px-3 py-2">
            <div className="flex min-w-0 items-baseline gap-2 text-sm">
              <span className="shrink-0 text-xs font-medium text-muted-foreground">
                {t("specs.currentWorkspaceLabel")}
              </span>
              <span className="min-w-0 truncate font-mono" title={initialPath}>
                {initialPath}
              </span>
            </div>
          </div>
        )}
        <div className="overflow-hidden rounded-lg border border-border">
          {rows.length > 0 && (
            <div className="flex items-center justify-between border-b border-border bg-muted/40 px-3 py-2">
              <span className="text-xs font-medium text-muted-foreground">{t("specs.configuredSources")}</span>
              <span className="text-xs tabular-nums text-muted-foreground">{rows.length}</span>
            </div>
          )}
          <div className="max-h-[50vh] overflow-y-auto">
            {rows.map((source, index) => (
              <SourceRow
                key={`${source.relativePath}-${index}`}
                source={source}
                onChange={(next) => setRows((current) => current.map((item, itemIndex) => itemIndex === index ? next : item))}
                onDelete={source.origin === "manual"
                  ? () => setRows((current) => current.filter((_, itemIndex) => itemIndex !== index))
                  : undefined}
              />
            ))}
            {rows.length === 0 && (
              <p className="p-6 text-center text-sm text-muted-foreground">
                {t("specs.noSources")}
              </p>
            )}
          </div>
        </div>
        {error && <p role="alert" data-selectable className="text-sm text-destructive">{error}</p>}
        <DialogFooter className="sm:justify-between">
          <Button type="button" variant="outline" disabled={busy !== null || initialPath === undefined} onClick={() => void addDirectory()}>
            <IconFolderPlus />
            {t("specs.addDirectory")}
          </Button>
          <div className="flex gap-2">
            <Button type="button" variant="outline" disabled={busy !== null} onClick={() => onOpenChange(false)}>
              {t("common.cancel")}
            </Button>
            <Button type="button" disabled={busy !== null} onClick={() => void save()}>
              {busy === "saving" ? t("common.saving") : t("common.save")}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function SourceRow({ source, onChange, onDelete }: {
  source: SpecSource;
  onChange: (source: SpecSource) => void;
  onDelete?: () => void;
}) {
  const { t } = useTranslation();
  const workflowValue = source.workflow.kind;
  const workflowEditable = source.origin === "manual";
  const setWorkflow = (kind: string | null) => {
    const workflow: SpecWorkflow = kind === "open_spec"
      ? { kind: "open_spec" }
      : kind === "superpowers"
        ? { kind: "superpowers" }
        : { kind: "custom", name: source.workflow.kind === "custom" ? source.workflow.name : t("specs.custom") };
    onChange({ ...source, workflow });
  };
  const setVisibility = (enabled: boolean) => {
    const visibility: SpecSourceVisibility = enabled ? "enabled" : "disabled";
    onChange({ ...source, visibility });
  };
  const customName = source.workflow.kind === "custom" ? source.workflow.name : t("specs.custom");

  return (
    <div
      className={`grid grid-cols-1 gap-3 border-b border-border px-3 py-3 last:border-b-0 sm:grid-cols-[minmax(0,1fr)_12rem_auto] sm:items-center sm:gap-4 ${source.visibility === "disabled" ? "opacity-60" : ""}`}
    >
      <div className="min-w-0">
        <p className="truncate font-mono text-sm" title={source.relativePath}>{source.relativePath}</p>
        <div className="mt-1.5 flex flex-wrap gap-1.5">
          <Badge variant="secondary" className="font-normal">{t(`specs.origin.${source.origin}`)}</Badge>
          <Badge
            variant="outline"
            className={source.availability === "missing" ? "border-amber-300 text-amber-800 dark:border-amber-700 dark:text-amber-300" : "font-normal"}
          >
            {t(`specs.availability.${source.availability}`)}
          </Badge>
        </div>
      </div>
      <div className="grid gap-1.5">
        <Select value={workflowValue} disabled={!workflowEditable} onValueChange={setWorkflow}>
          <SelectTrigger className="w-full">
            <SelectValue placeholder={workflowKindLabel(workflowValue, customName, t)}>
              {workflowKindLabel(workflowValue, customName, t)}
            </SelectValue>
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="open_spec">OpenSpec</SelectItem>
            <SelectItem value="superpowers">Superpowers</SelectItem>
            <SelectItem value="custom">{t("specs.custom")}</SelectItem>
          </SelectContent>
        </Select>
        {source.workflow.kind === "custom" && (
          <Input
            className="h-8"
            aria-label={t("specs.customName")}
            value={source.workflow.name}
            disabled={!workflowEditable}
            onChange={(event) => onChange({ ...source, workflow: { kind: "custom", name: event.target.value } })}
          />
        )}
      </div>
      <div className="flex items-center justify-end gap-1 sm:min-w-16 sm:justify-center">
        <Switch
          checked={source.visibility === "enabled"}
          aria-label={t("specs.toggleSource")}
          onCheckedChange={setVisibility}
        />
        {onDelete && (
          <Button size="icon-sm" variant="ghost" className="text-muted-foreground" aria-label={t("specs.removeSource")} onClick={onDelete}>
            <IconTrash />
          </Button>
        )}
      </div>
    </div>
  );
}
