import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
  Button,
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
  Switch,
  toast,
} from "@ora/ui";
import {
  IconLoader2,
  IconPencil,
  IconPlus,
  IconTrash,
} from "@tabler/icons-react";
import type {
  MarketplaceArtifactRetrieval,
  MarketplaceArtifactRetrievalUpdate,
  MarketplaceSource,
} from "@ora/contracts";
import { useContractErrorToast } from "../../i18n/use-contract-error-toast";
import {
  useAddMarketplaceSource,
  useDeleteMarketplaceSource,
  useUpdateMarketplaceSource,
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
  const showContractError = useContractErrorToast();
  const sourcesQuery = useMarketplaceSources();
  const addSource = useAddMarketplaceSource();
  const deleteSource = useDeleteMarketplaceSource();
  const updateSource = useUpdateMarketplaceSource();
  const [url, setUrl] = useState("");
  const [branch, setBranch] = useState("main");
  const [editing, setEditing] = useState<MarketplaceSource | null>(null);

  const sources = sourcesQuery.data?.sources ?? [];

  const handleAdd = () => {
    const nextUrl = url.trim();
    const nextBranch = branch.trim();
    if (nextUrl === "" || nextBranch === "") return;
    addSource.mutate(
      { url: nextUrl, branch: nextBranch, useProxy: false },
      {
        onSuccess: () => {
          setUrl("");
          setBranch("main");
          toast.success(t("settings.plugins.sourceAdded"));
        },
        onError: (cause) =>
          showContractError(cause, t("settings.plugins.sourceAddFailed")),
      },
    );
  };

  const patchSource = (
    source: MarketplaceSource,
    patch: Partial<
      Pick<MarketplaceSource, "url" | "branch" | "useProxy" | "enabled">
    > & { artifactRetrieval?: MarketplaceArtifactRetrievalUpdate },
    onSuccess?: () => void,
  ) => {
    updateSource.mutate(
      {
        url: source.url,
        newUrl: patch.url ?? source.url,
        branch: patch.branch ?? source.branch,
        useProxy: patch.useProxy ?? source.useProxy,
        enabled: patch.enabled ?? source.enabled,
        artifactRetrieval:
          patch.artifactRetrieval ??
          preservedArtifactRetrieval(source.artifactRetrieval),
      },
      {
        onSuccess: () => {
          onSuccess?.();
        },
        onError: (cause) =>
          showContractError(cause, t("settings.plugins.sourceUpdateFailed")),
      },
    );
  };

  return (
    <div className="space-y-5">
      <Breadcrumb>
        <BreadcrumbList>
          <BreadcrumbItem>
            <BreadcrumbLink render={<button type="button" onClick={onBack} />}>
              {t("settings.plugins.title")}
            </BreadcrumbLink>
          </BreadcrumbItem>
          <BreadcrumbSeparator />
          <BreadcrumbItem>
            <BreadcrumbPage>
              {t("settings.plugins.manageSources")}
            </BreadcrumbPage>
          </BreadcrumbItem>
        </BreadcrumbList>
      </Breadcrumb>

      <header>
        <h2 className="text-lg font-semibold">
          {t("settings.plugins.manageSources")}
        </h2>
      </header>

      <form
        className="flex flex-col gap-3 rounded-lg border border-border/70 bg-muted/25 p-3 sm:flex-row sm:items-center"
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
          className="min-w-0 flex-1 bg-background"
        />
        <Input
          value={branch}
          onChange={(event) => setBranch(event.target.value)}
          placeholder={t("settings.plugins.sourceBranch")}
          aria-label={t("settings.plugins.sourceBranch")}
          className="bg-background sm:w-40"
        />
        <Button
          type="submit"
          variant="outline"
          size="sm"
          className="shrink-0"
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
        <div className="divide-y divide-border overflow-hidden rounded-lg border border-border">
          {sources.map((source) => (
            <div
              key={source.url}
              className="flex items-center gap-3 px-3 py-3 sm:px-4"
            >
              <span
                className={`min-w-0 flex-1 ${source.enabled ? "" : "opacity-50"}`}
              >
                <span className="block truncate text-sm font-medium">
                  {source.url}
                </span>
                <span className="block truncate text-xs text-muted-foreground">
                  {t("settings.plugins.sourceBranch")}:{" "}
                  <span className="font-mono">{source.branch}</span>
                </span>
              </span>
              <label className="flex items-center gap-2">
                <Switch
                  checked={source.useProxy}
                  title={t("settings.plugins.sourceUseProxy")}
                  disabled={updateSource.isPending}
                  onCheckedChange={(checked) =>
                    patchSource(source, { useProxy: checked })
                  }
                  aria-label={`${t("settings.plugins.sourceUseProxy")}: ${source.url}`}
                />
                <span className="text-xs text-muted-foreground">
                  {t("settings.plugins.sourceUseProxy")}
                </span>
              </label>
              <Button
                variant="outline"
                size="sm"
                className="shrink-0"
                disabled={updateSource.isPending}
                onClick={() =>
                  patchSource(source, { enabled: !source.enabled }, () =>
                    toast.success(
                      source.enabled
                        ? t("settings.plugins.sourceDisabled")
                        : t("settings.plugins.sourceEnabled"),
                    ),
                  )
                }
                aria-label={`${
                  source.enabled
                    ? t("settings.plugins.disableSource")
                    : t("settings.plugins.enableSource")
                }: ${source.url}`}
              >
                {source.enabled
                  ? t("settings.plugins.disableSource")
                  : t("settings.plugins.enableSource")}
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                className="shrink-0 text-muted-foreground"
                disabled={updateSource.isPending}
                onClick={() => setEditing(source)}
                aria-label={`${t("settings.plugins.editSource")}: ${source.url}`}
              >
                <IconPencil />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                className="shrink-0 text-muted-foreground hover:text-destructive"
                disabled={deleteSource.isPending}
                onClick={() =>
                  deleteSource.mutate(
                    { url: source.url },
                    {
                      onSuccess: () =>
                        toast.success(t("settings.plugins.sourceRemoved")),
                      onError: (cause) =>
                        showContractError(
                          cause,
                          t("settings.plugins.sourceRemoveFailed"),
                        ),
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

      {editing !== null && (
        <SourceEditDialog
          source={editing}
          pending={updateSource.isPending}
          onOpenChange={(open) => {
            if (!open) setEditing(null);
          }}
          onSave={(nextUrl, nextBranch, artifactRetrieval) =>
            patchSource(
              editing,
              { url: nextUrl, branch: nextBranch, artifactRetrieval },
              () => {
                setEditing(null);
                toast.success(t("settings.plugins.sourceUpdated"));
              },
            )
          }
        />
      )}
    </div>
  );
}

interface SourceEditDialogProps {
  source: MarketplaceSource;
  pending: boolean;
  onOpenChange: (open: boolean) => void;
  onSave: (
    url: string,
    branch: string,
    artifactRetrieval: MarketplaceArtifactRetrievalUpdate,
  ) => void;
}

/** Edits one marketplace source's Git identity and artifact retrieval policy. */
function SourceEditDialog({
  source,
  pending,
  onOpenChange,
  onSave,
}: SourceEditDialogProps) {
  const { t } = useTranslation();
  const [url, setUrl] = useState(source.url);
  const [branch, setBranch] = useState(source.branch);
  const [retrievalType, setRetrievalType] = useState(
    source.artifactRetrieval.type,
  );
  const [endpoint, setEndpoint] = useState(
    source.artifactRetrieval.type === "s3_sigv4"
      ? source.artifactRetrieval.endpoint
      : "",
  );
  const [bucket, setBucket] = useState(
    source.artifactRetrieval.type === "s3_sigv4"
      ? source.artifactRetrieval.bucket
      : "",
  );
  const [region, setRegion] = useState(
    source.artifactRetrieval.type === "s3_sigv4"
      ? source.artifactRetrieval.region
      : "",
  );
  const [accessKeyId, setAccessKeyId] = useState("");
  const [secretAccessKey, setSecretAccessKey] = useState("");
  const keepsCredentials =
    source.artifactRetrieval.type === "s3_sigv4" &&
    accessKeyId === "" &&
    secretAccessKey === "";
  const hasReplacementCredentials =
    accessKeyId.trim() !== "" && secretAccessKey.trim() !== "";
  const canSave =
    url.trim() !== "" &&
    branch.trim() !== "" &&
    (retrievalType === "direct_https" ||
      (endpoint.trim() !== "" &&
        bucket.trim() !== "" &&
        region.trim() !== "" &&
        (keepsCredentials || hasReplacementCredentials)));

  const artifactRetrieval = (): MarketplaceArtifactRetrievalUpdate => {
    if (retrievalType === "direct_https") return { type: "direct_https" };
    return {
      type: "s3_sigv4",
      endpoint: endpoint.trim(),
      bucket: bucket.trim(),
      region: region.trim(),
      credentials: keepsCredentials
        ? { action: "preserve" }
        : {
            action: "replace",
            accessKeyId: accessKeyId.trim(),
            secretAccessKey: secretAccessKey.trim(),
          },
    };
  };

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("settings.plugins.editSource")}</DialogTitle>
          <DialogDescription>
            {t("settings.plugins.editSourceDescription")}
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <label className="space-y-1.5">
            <span className="text-sm font-medium">
              {t("settings.plugins.sourceUrl")}
            </span>
            <Input
              value={url}
              onChange={(event) => setUrl(event.target.value)}
              aria-label={t("settings.plugins.sourceUrl")}
              autoComplete="off"
            />
          </label>
          <label className="space-y-1.5">
            <span className="text-sm font-medium">
              {t("settings.plugins.sourceBranch")}
            </span>
            <Input
              value={branch}
              onChange={(event) => setBranch(event.target.value)}
              aria-label={t("settings.plugins.sourceBranch")}
              autoComplete="off"
            />
          </label>
          <label className="space-y-1.5">
            <span className="text-sm font-medium">
              {t("settings.plugins.artifactRetrieval")}
            </span>
            <Select
              value={retrievalType}
              onValueChange={(value) =>
                setRetrievalType(value as MarketplaceArtifactRetrieval["type"])
              }
            >
              <SelectTrigger
                className="w-full"
                aria-label={t("settings.plugins.artifactRetrieval")}
              >
                <span className="flex-1 text-left">
                  {retrievalType === "direct_https"
                    ? t("settings.plugins.artifactRetrievalDirectHttps")
                    : t("settings.plugins.artifactRetrievalS3SigV4")}
                </span>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="direct_https">
                  {t("settings.plugins.artifactRetrievalDirectHttps")}
                </SelectItem>
                <SelectItem value="s3_sigv4">
                  {t("settings.plugins.artifactRetrievalS3SigV4")}
                </SelectItem>
              </SelectContent>
            </Select>
          </label>
          {retrievalType === "s3_sigv4" && (
            <div className="space-y-3 rounded-md border border-border/70 p-3">
              <SourceField
                label={t("settings.plugins.s3Endpoint")}
                value={endpoint}
                onChange={setEndpoint}
              />
              <SourceField
                label={t("settings.plugins.s3Bucket")}
                value={bucket}
                onChange={setBucket}
              />
              <SourceField
                label={t("settings.plugins.s3Region")}
                value={region}
                onChange={setRegion}
              />
              <SourceField
                label={t("settings.plugins.s3AccessKeyId")}
                value={accessKeyId}
                onChange={setAccessKeyId}
                autoComplete="off"
              />
              <SourceField
                label={t("settings.plugins.s3SecretAccessKey")}
                value={secretAccessKey}
                onChange={setSecretAccessKey}
                type="password"
                autoComplete="new-password"
              />
              {source.artifactRetrieval.type === "s3_sigv4" && (
                <p className="text-xs text-muted-foreground">
                  {t("settings.plugins.s3CredentialsPreserved")}
                </p>
              )}
            </div>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel")}
          </Button>
          <Button
            disabled={pending || !canSave}
            onClick={() =>
              onSave(url.trim(), branch.trim(), artifactRetrieval())
            }
          >
            {pending ? t("settings.plugins.savingSource") : t("common.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface SourceFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  type?: "password";
  autoComplete?: string;
}

/** Renders one labeled marketplace source field with consistent credential behavior. */
function SourceField({
  label,
  value,
  onChange,
  type,
  autoComplete,
}: SourceFieldProps) {
  return (
    <label className="space-y-1.5">
      <span className="text-sm font-medium">{label}</span>
      <Input
        type={type}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        aria-label={label}
        autoComplete={autoComplete}
      />
    </label>
  );
}

/** Converts a redacted source projection into an update that keeps any persisted credentials. */
function preservedArtifactRetrieval(
  retrieval: MarketplaceArtifactRetrieval,
): MarketplaceArtifactRetrievalUpdate {
  if (retrieval.type === "direct_https") return retrieval;
  return {
    ...retrieval,
    credentials: { action: "preserve" },
  };
}
