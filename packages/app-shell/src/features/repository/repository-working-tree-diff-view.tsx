import type {
  RepositoryChangeSelection,
  RepositoryConflictSide,
  RepositorySyncOperation,
  RepositoryWorkingTree,
  RepositoryWorkingTreeFile,
} from "@ora/contracts";
import { Badge, Button, Textarea } from "@ora/ui";
import { IconFileDiff, IconGitCommit, IconGitMerge, IconMinus, IconPlus } from "@tabler/icons-react";
import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { localizeContractError } from "../../i18n/contract-error";
import {
  useCommitRepositoryChanges,
  useResolveRepositoryConflict,
  useStageRepositoryChanges,
  useUnstageRepositoryChanges,
} from "../../state/hooks/use-repository-change-mutations";
import { useRepositoryWorkingTreeDiff } from "../../state/hooks/use-repository-working-tree-diff";
import { RepositoryPatchView } from "./repository-commit-diff-view";

const CONFLICT_STATUSES = new Set(["DD", "AU", "UD", "UA", "DU", "AA", "UU"]);

interface RepositoryWorkingTreeDiffViewProps {
  projectId: string;
  workingTree?: RepositoryWorkingTree;
  syncOperation?: RepositorySyncOperation | null;
}

/** Renders the main repository checkout patch and its stage/commit controls. */
export function RepositoryWorkingTreeDiffView({
  projectId,
  workingTree,
  syncOperation,
}: RepositoryWorkingTreeDiffViewProps) {
  const { t } = useTranslation();
  const diffQuery = useRepositoryWorkingTreeDiff(projectId);
  const stageChanges = useStageRepositoryChanges();
  const unstageChanges = useUnstageRepositoryChanges();
  const resolveConflict = useResolveRepositoryConflict();
  const commitChanges = useCommitRepositoryChanges();
  const [commitMessage, setCommitMessage] = useState("");
  const [actionNotice, setActionNotice] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const files = workingTree?.files ?? [];
  const conflictFiles = files.filter((file) => isConflictedStatus(file.status));
  const stagedFiles = files.filter((file) => !isConflictedStatus(file.status) && file.staged);
  const unstagedFiles = files.filter((file) => !isConflictedStatus(file.status) && file.unstaged);
  const changeActionPending = stageChanges.isPending
    || unstageChanges.isPending
    || resolveConflict.isPending;

  const runChangeAction = async (
    mutation: typeof stageChanges,
    selection: RepositoryChangeSelection,
  ) => {
    setActionNotice(null);
    setActionError(null);
    try {
      await mutation.mutateAsync({ projectId, selection });
      setActionNotice(t("repository.changesUpdated"));
    } catch (error) {
      setActionError(localizeContractError(error, t));
    }
  };

  const handleCommit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setActionNotice(null);
    setActionError(null);
    try {
      await commitChanges.mutateAsync({ projectId, message: commitMessage });
      setCommitMessage("");
      setActionNotice(t("repository.commitSucceeded"));
    } catch (error) {
      setActionError(localizeContractError(error, t));
    }
  };

  const handleConflictResolution = async (
    file: RepositoryWorkingTreeFile,
    side: RepositoryConflictSide,
  ) => {
    setActionNotice(null);
    setActionError(null);
    try {
      await resolveConflict.mutateAsync({ projectId, path: file.path, side });
      setActionNotice(t("repository.conflictResolved"));
    } catch (error) {
      setActionError(localizeContractError(error, t));
    }
  };

  return (
    <section className="flex h-full min-h-0 flex-col bg-background">
      <header className="flex min-h-12 shrink-0 items-center gap-2 border-b border-border px-3">
        <IconFileDiff className="size-4 text-muted-foreground" />
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold">{t("repository.mainChanges")}</p>
          <p className="truncate text-[11px] text-muted-foreground">
            {t("repository.mainChangesDescription")}
          </p>
        </div>
        {workingTree !== undefined && (
          <div className="ml-auto flex shrink-0 items-center gap-1.5 text-[10px]">
            <Badge variant="secondary">{workingTree.changedFiles}</Badge>
            <span className="text-muted-foreground">{t("repository.changedFiles")}</span>
            {workingTree.stagedFiles > 0 && (
              <Badge variant="outline">{t("diff.scopeStaged")}: {workingTree.stagedFiles}</Badge>
            )}
            {workingTree.unstagedFiles > 0 && (
              <Badge variant="outline">{t("diff.scopeUnstaged")}: {workingTree.unstagedFiles}</Badge>
            )}
            {workingTree.untrackedFiles > 0 && (
              <Badge variant="outline">{t("repository.untracked")}: {workingTree.untrackedFiles}</Badge>
            )}
          </div>
        )}
      </header>

      <div className="shrink-0 border-b border-border bg-muted/10 p-3">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-xs font-semibold">{t("repository.changeActions")}</span>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={changeActionPending || unstagedFiles.length === 0}
            onClick={() => void runChangeAction(stageChanges, { kind: "all" })}
          >
            <IconPlus />
            {t("repository.stageAll")}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="sm"
            disabled={changeActionPending || stagedFiles.length === 0}
            onClick={() => void runChangeAction(unstageChanges, { kind: "all" })}
          >
            <IconMinus />
            {t("repository.unstageAll")}
          </Button>
        </div>

        {conflictFiles.length > 0 && (
          <RepositoryConflictGroup
            files={conflictFiles}
            operation={syncOperation}
            disabled={changeActionPending}
            onResolve={(file, side) => void handleConflictResolution(file, side)}
          />
        )}

        {(stagedFiles.length > 0 || unstagedFiles.length > 0) && (
          <div className="mt-3 grid max-h-56 gap-3 overflow-auto md:grid-cols-2">
            {stagedFiles.length > 0 && (
              <RepositoryChangeGroup
                title={t("repository.stagedChanges")}
                files={stagedFiles}
                actionLabel={t("repository.unstage")}
                disabled={changeActionPending}
                onAction={(file) => void runChangeAction(unstageChanges, { kind: "paths", paths: [file.path] })}
              />
            )}
            {unstagedFiles.length > 0 && (
              <RepositoryChangeGroup
                title={t("repository.unstagedChanges")}
                files={unstagedFiles}
                actionLabel={t("repository.stage")}
                disabled={changeActionPending}
                onAction={(file) => void runChangeAction(stageChanges, { kind: "paths", paths: [file.path] })}
              />
            )}
          </div>
        )}

        <form className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-end" onSubmit={handleCommit}>
          <label className="min-w-0 flex-1 text-xs font-medium">
            {t("repository.commitMessage")}
            <Textarea
              rows={2}
              value={commitMessage}
              placeholder={t("repository.commitMessagePlaceholder")}
              onChange={(event) => setCommitMessage(event.target.value)}
              className="mt-1 min-h-14 resize-y text-sm"
            />
          </label>
          <Button
            type="submit"
            size="sm"
            disabled={
              commitChanges.isPending
              || syncOperation !== undefined && syncOperation !== null
              || stagedFiles.length === 0
              || commitMessage.trim().length === 0
            }
          >
            <IconGitCommit />
            {commitChanges.isPending ? t("repository.committing") : t("repository.commit")}
          </Button>
        </form>
        {actionNotice !== null && (
          <p className="mt-2 text-xs text-emerald-600 dark:text-emerald-400" role="status">
            {actionNotice}
          </p>
        )}
        {actionError !== null && (
          <p className="mt-2 text-xs text-destructive" role="alert">
            {actionError}
          </p>
        )}
      </div>

      {diffQuery.isPending ? (
        <div className="flex min-h-0 flex-1 items-center justify-center p-6 text-sm text-muted-foreground">
          {t("repository.loadingChanges")}
        </div>
      ) : diffQuery.error !== null ? (
        <div className="flex min-h-0 flex-1 items-center justify-center p-6 text-center text-sm text-destructive">
          <div>
            <p className="font-medium">{t("repository.changesError")}</p>
            <p className="mt-2 text-xs">{localizeContractError(diffQuery.error, t)}</p>
          </div>
        </div>
      ) : (
        <RepositoryPatchView
          patch={diffQuery.data?.patch ?? ""}
          emptyMessage={t("repository.noWorkingTreeChanges")}
          className="min-h-0 flex-1 overflow-auto bg-background"
        />
      )}
    </section>
  );
}

interface RepositoryConflictGroupProps {
  files: RepositoryWorkingTreeFile[];
  operation?: RepositorySyncOperation | null;
  disabled: boolean;
  onResolve: (file: RepositoryWorkingTreeFile, side: RepositoryConflictSide) => void;
}

/** Presents safe one-click conflict choices while keeping the active sync operation resumable. */
function RepositoryConflictGroup({
  files,
  operation,
  disabled,
  onResolve,
}: RepositoryConflictGroupProps) {
  const { t } = useTranslation();
  const oursLabel = operation === "rebase"
    ? t("repository.keepUpstream")
    : t("repository.keepCurrent");
  const theirsLabel = operation === "rebase"
    ? t("repository.keepLocalCommit")
    : t("repository.keepIncoming");

  return (
    <div className="mt-3 overflow-hidden rounded-md border border-destructive/30 bg-destructive/5">
      <div className="border-b border-destructive/20 px-2.5 py-2">
        <div className="flex items-center gap-2 text-[11px] font-semibold text-destructive">
          <IconGitMerge className="size-3.5" />
          {t("repository.conflictResolution")}
          <Badge variant="destructive" className="ml-auto text-[10px]">{files.length}</Badge>
        </div>
        <p className="mt-1 text-[11px] text-muted-foreground">
          {t("repository.conflictDescription")}
        </p>
      </div>
      {files.map((file) => (
        <div
          key={`${file.status}:${file.path}`}
          className="flex flex-wrap items-center gap-2 border-b border-destructive/20 px-2.5 py-2 last:border-b-0"
        >
          <span className="w-5 shrink-0 font-mono text-[10px] text-destructive">{file.status}</span>
          <span className="min-w-0 flex-1 truncate font-mono text-[11px]" title={file.path}>
            {file.path}
          </span>
          <Button
            type="button"
            variant="outline"
            size="xs"
            disabled={disabled}
            onClick={() => onResolve(file, "ours")}
          >
            {oursLabel}
          </Button>
          <Button
            type="button"
            variant="outline"
            size="xs"
            disabled={disabled}
            onClick={() => onResolve(file, "theirs")}
          >
            {theirsLabel}
          </Button>
        </div>
      ))}
    </div>
  );
}

interface RepositoryChangeGroupProps {
  title: string;
  files: RepositoryWorkingTreeFile[];
  actionLabel: string;
  disabled: boolean;
  onAction: (file: RepositoryWorkingTreeFile) => void;
}

/** Renders one staged or unstaged path group with a narrow per-file action. */
function RepositoryChangeGroup({
  title,
  files,
  actionLabel,
  disabled,
  onAction,
}: RepositoryChangeGroupProps) {
  return (
    <div className="overflow-hidden rounded-md border border-border bg-background">
      <div className="border-b border-border px-2.5 py-1.5 text-[11px] font-semibold">
        {title} ({files.length})
      </div>
      <div>
        {files.map((file) => (
          <div key={`${file.status}:${file.path}`} className="flex items-center gap-2 border-b border-border px-2.5 py-1.5 last:border-b-0">
            <span className="w-5 shrink-0 font-mono text-[10px] text-muted-foreground">{file.status}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-[11px]" title={file.path}>
              {file.path}
            </span>
            <Button type="button" variant="ghost" size="xs" disabled={disabled} onClick={() => onAction(file)}>
              {actionLabel}
            </Button>
          </div>
        ))}
      </div>
    </div>
  );
}

/** Recognizes Git's porcelain-v2 unmerged status codes without treating ordinary edits as conflicts. */
function isConflictedStatus(status: string): boolean {
  return CONFLICT_STATUSES.has(status);
}
