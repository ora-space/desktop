import { useState, type ReactNode } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Button } from "@ora/ui";
import { IconFolder, IconRefresh } from "@tabler/icons-react";
import { refreshWorkspaceDirectories } from "../../state/data/workspace";

/**
 * Replaces the Changes panel for a workspace whose folder is not a Git repository.
 *
 * There is no diff to load there: agents write straight into the folder. Saying so is clearer
 * than a load error, and the panel toolbar stays so the user can switch to Files. The re-check
 * re-probes the folder, so running `git init` while Ora stays focused takes effect at once.
 */
export function TaskDiffNotGitState({ toolbar }: { toolbar?: ReactNode }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [checking, setChecking] = useState(false);
  const recheck = () => {
    setChecking(true);
    void refreshWorkspaceDirectories(queryClient).finally(() =>
      setChecking(false),
    );
  };

  return (
    <section
      className="flex h-full min-h-0 flex-1 flex-col overflow-hidden bg-background"
      aria-label={t("diff.taskChanges")}
      data-workspace-git="notRepository"
    >
      {toolbar !== undefined && (
        <header className="flex min-h-12 shrink-0 items-center justify-end gap-0.5 border-b border-border px-3 py-2 sm:px-4">
          {toolbar}
        </header>
      )}
      <div className="flex min-h-0 flex-1 items-center justify-center p-8">
        <div className="max-w-sm text-center">
          <IconFolder className="mx-auto size-6 text-muted-foreground" />
          <h2 className="mt-3 text-sm font-semibold">
            {t("diff.notGitTitle")}
          </h2>
          <p className="mt-1 text-xs leading-5 text-muted-foreground">
            {t("diff.notGitDetail")}
          </p>
          <div className="mt-4">
            <Button
              size="sm"
              variant="outline"
              disabled={checking}
              onClick={recheck}
            >
              <IconRefresh />
              {checking ? t("diff.notGitRechecking") : t("diff.notGitRecheck")}
            </Button>
          </div>
        </div>
      </div>
    </section>
  );
}
