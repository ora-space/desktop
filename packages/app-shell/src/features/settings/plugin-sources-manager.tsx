import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Input, toast } from "@ora/ui";
import { IconLoader2, IconPlus, IconTrash } from "@tabler/icons-react";
import { localizeContractError } from "../../i18n/contract-error";
import {
  useAddMarketplaceSource,
  useDeleteMarketplaceSource,
  useMarketplaceSources,
} from "../../state/hooks/use-marketplace-sources";

/**
 * Lists, adds, and removes the backend-persisted marketplace Git sources.
 *
 * Sources are shown in backend precedence order; the first source wins when
 * two sources publish the same plugin id. Adding and removing writes the
 * configuration immediately, while the cached registry index is refreshed
 * through the normal marketplace sync button owned by `PluginsSettings`.
 */
export function PluginSourcesManager({ onBack }: { onBack: () => void }) {
  const { t } = useTranslation();
  const sourcesQuery = useMarketplaceSources();
  const addSource = useAddMarketplaceSource();
  const deleteSource = useDeleteMarketplaceSource();
  const [url, setUrl] = useState("");
  const [branch, setBranch] = useState("main");

  const sources = sourcesQuery.data?.sources ?? [];

  const handleAdd = () => {
    const nextUrl = url.trim();
    const nextBranch = branch.trim();
    if (nextUrl === "" || nextBranch === "") return;
    addSource.mutate(
      { url: nextUrl, branch: nextBranch },
      {
        onSuccess: () => {
          setUrl("");
          setBranch("main");
          toast.success(t("settings.plugins.sourceAdded"));
        },
        onError: (cause) =>
          toast.error(t("settings.plugins.sourceAddFailed"), {
            description: localizeContractError(cause, t),
          }),
      },
    );
  };

  return (
    <div className="space-y-5">
      <header className="flex items-center gap-3">
        <Button variant="ghost" size="sm" onClick={onBack}>
          {t("settings.plugins.back")}
        </Button>
        <div className="min-w-0 flex-1">
          <h2 className="text-lg font-semibold">
            {t("settings.plugins.manageSources")}
          </h2>
          <p className="mt-1 text-sm leading-6 text-muted-foreground">
            {t("settings.plugins.manageSourcesDescription")}
          </p>
        </div>
      </header>

      <form
        className="flex flex-col gap-2 sm:flex-row sm:items-center"
        onSubmit={(event) => {
          event.preventDefault();
          handleAdd();
        }}
      >
        <Input
          value={url}
          onChange={(event) => setUrl(event.target.value)}
          placeholder={t("settings.plugins.sourceUrl")}
          aria-label={t("settings.plugins.sourceUrl")}
          className="min-w-0 flex-1"
        />
        <Input
          value={branch}
          onChange={(event) => setBranch(event.target.value)}
          placeholder={t("settings.plugins.sourceBranch")}
          aria-label={t("settings.plugins.sourceBranch")}
          className="sm:w-44"
        />
        <Button
          type="submit"
          variant="outline"
          size="sm"
          disabled={addSource.isPending}
        >
          {addSource.isPending ? (
            <IconLoader2 className="animate-spin" />
          ) : (
            <IconPlus />
          )}
          {t("settings.plugins.addSource")}
        </Button>
      </form>

      {sources.length === 0 ? (
        <p className="py-10 text-center text-sm text-muted-foreground">
          {t("settings.plugins.emptySources")}
        </p>
      ) : (
        <div className="divide-y divide-border border-y border-border">
          {sources.map((source) => (
            <div key={source.url} className="flex items-center gap-3 py-3">
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm font-medium">
                  {source.url}
                </span>
                <span className="block truncate text-xs text-muted-foreground">
                  {t("settings.plugins.sourceBranch")}: {source.branch}
                </span>
              </span>
              <Button
                variant="ghost"
                size="sm"
                className="shrink-0 text-muted-foreground"
                disabled={deleteSource.isPending}
                onClick={() =>
                  deleteSource.mutate(
                    { url: source.url },
                    {
                      onSuccess: () =>
                        toast.success(t("settings.plugins.sourceRemoved")),
                      onError: (cause) =>
                        toast.error(t("settings.plugins.sourceRemoveFailed"), {
                          description: localizeContractError(cause, t),
                        }),
                    },
                  )
                }
                aria-label={`${t("settings.plugins.deleteSource")}: ${source.url}`}
              >
                <IconTrash />
              </Button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
