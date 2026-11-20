import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useQueryClient } from "@tanstack/react-query";
import { IconFolderX } from "@tabler/icons-react";
import { refreshWorkspaceDirectories } from "../../state/data/workspace";

/**
 * Explains that the selected workspace's folder is gone and offers a re-probe.
 *
 * Without this, a deleted project still opens and accepts a new chat, but every agent shows
 * an empty model list and the first send fails — which reads as a plugin fault. Naming the
 * folder points the user at the actual cause; the re-check lets a restored folder recover
 * without restarting Ora.
 */
export function WorkspaceDirectoryBanner({
  missingPath,
}: {
  missingPath: string | null;
}) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [checking, setChecking] = useState(false);

  if (missingPath === null) return null;
  const recheck = () => {
    setChecking(true);
    void refreshWorkspaceDirectories(queryClient).finally(() =>
      setChecking(false),
    );
  };
  return (
    <div
      role="alert"
      data-workspace-directory="missing"
      className="mx-3 mb-2 flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs sm:mx-4"
    >
      <IconFolderX
        className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400"
        aria-hidden="true"
      />
      <div className="min-w-0 flex-1">
        <p className="font-medium text-amber-700 dark:text-amber-300">
          {t("workspace.directoryMissing.title")}
        </p>
        <p className="mt-0.5 break-all text-muted-foreground">
          {t("workspace.directoryMissing.description", { path: missingPath })}
        </p>
      </div>
      <button
        type="button"
        disabled={checking}
        onClick={recheck}
        className="shrink-0 rounded-md border border-amber-500/40 px-2.5 py-1 font-medium text-amber-700 transition-colors hover:bg-amber-500/10 disabled:opacity-50 dark:text-amber-300"
      >
        {checking
          ? t("workspace.directoryMissing.rechecking")
          : t("workspace.directoryMissing.recheck")}
      </button>
    </div>
  );
}
