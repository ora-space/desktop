import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useMemo,
  useState,
} from "react";
import type { SpecTarget } from "@ora/contracts";
import {
  Button,
  Input,
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  ScrollArea,
} from "@ora/ui";
import { IconCode, IconEye, IconSearch } from "@tabler/icons-react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import type { Components } from "react-markdown";
import { useContractsClient } from "../../contracts-client-context";
import { localizeContractError } from "../../i18n/contract-error";
import { queryKeys } from "../../state/hooks/query-keys";
import { useTaskWorkspace } from "../../state/hooks/use-task-workspace";
import { MarkdownDocument } from "../chat/markdown-message";
import { WorkspaceFileViewer } from "../files/workspace-file-viewer";
import { watchWorkspaceContinuously } from "../files/workspace-watch";
import { invalidateSpecQueries, resolveMarkdownLink } from "./spec-query-utils";
import { SpecSourceDialog } from "./spec-source-dialog";
import { SpecTree } from "./spec-tree";

export interface SpecsContentHandle {
  refresh: () => Promise<void>;
  isRefreshing: boolean;
  clearSelection: () => void;
}

interface SpecsContentProps {
  projectId: string;
  projectRootPath: string;
  taskId?: string;
  onRefreshingChange?: (refreshing: boolean) => void;
}

/** Presents project/worktree specification documents as a read-only review surface. */
export const SpecsContent = forwardRef<SpecsContentHandle, SpecsContentProps>(function SpecsContent(
  { projectId, projectRootPath, taskId, onRefreshingChange },
  ref,
) {
  const { t } = useTranslation();
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const target = useMemo<SpecTarget>(
    () => taskId === undefined
      ? { kind: "project", projectId }
      : { kind: "task", taskId },
    [projectId, taskId],
  );
  const targetKey = taskId === undefined ? `project:${projectId}` : `task:${taskId}`;
  const workspaceQuery = useTaskWorkspace(taskId);
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [mode, setMode] = useState<"preview" | "source">("preview");
  const [filter, setFilter] = useState("");
  const [debouncedFilter, setDebouncedFilter] = useState("");

  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedFilter(filter.trim().toLowerCase()), 200);
    return () => window.clearTimeout(timer);
  }, [filter]);

  const catalogQuery = useQuery({
    queryKey: queryKeys.specCatalog(projectId, targetKey),
    queryFn: ({ signal }) => client.spec.catalog({ target }, { signal }),
  });
  const documents = useMemo(() => {
    const all = catalogQuery.data?.documents ?? [];
    if (debouncedFilter === "") return all;
    return all.filter((document) => document.relativePath.toLowerCase().includes(debouncedFilter));
  }, [catalogQuery.data?.documents, debouncedFilter]);
  const selectedDocument = catalogQuery.data?.documents.find(
    (document) => document.relativePath === selectedPath,
  );
  // Catalog membership is the authorization boundary, so a removed document becomes
  // unselected immediately without a synchronization effect or a stale read request.
  const activeSelectedPath = selectedDocument?.relativePath ?? null;

  const documentQuery = useQuery({
    queryKey: queryKeys.specDocument(projectId, targetKey, activeSelectedPath ?? ""),
    queryFn: ({ signal }) => client.spec.read({ target, relativePath: activeSelectedPath! }, { signal }),
    enabled: activeSelectedPath !== null,
  });

  useEffect(() => {
    const controller = new AbortController();
    void watchWorkspaceContinuously({
      signal: controller.signal,
      openStream: (signal) => client.spec.watch({ target }, { signal }),
      onBatch: (batch) => invalidateSpecQueries(
        queryClient,
        projectId,
        targetKey,
        batch.changes,
      ),
    });
    return () => controller.abort();
  }, [client, projectId, queryClient, target, targetKey]);

  const catalogPaths = useMemo(
    () => new Set((catalogQuery.data?.documents ?? []).map((document) => document.relativePath)),
    [catalogQuery.data?.documents],
  );
  const markdownComponents: Components = {
    a: ({ href, children, ...props }) => {
      const destination = href === undefined || activeSelectedPath === null
        ? null
        : resolveMarkdownLink(activeSelectedPath, href);
      const internal = destination !== null && catalogPaths.has(destination);
      return (
        <a
          {...props}
          className="font-medium text-primary underline decoration-primary/45 underline-offset-4 transition-colors hover:decoration-primary focus-visible:rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
          href={internal ? `#${destination}` : href}
          rel="noopener noreferrer"
          target={href?.startsWith("http://") || href?.startsWith("https://") ? "_blank" : undefined}
          onClick={(event) => {
            if (internal) {
              event.preventDefault();
              setSelectedPath(destination);
            } else if (!href?.startsWith("http://") && !href?.startsWith("https://")) {
              event.preventDefault();
            }
          }}
        >
          {children}
        </a>
      );
    },
    img: ({ alt }) => <span className="text-sm text-muted-foreground">[{t("specs.localImageBlocked")}: {alt ?? ""}]</span>,
  };

  const workflowLabel = selectedDocument === undefined
    ? null
    : selectedDocument.workflow.kind === "open_spec"
      ? "OpenSpec"
      : selectedDocument.workflow.kind === "superpowers"
        ? "Superpowers"
        : selectedDocument.workflow.name;
  const refresh = async () => {
    const invalidations = [
      queryClient.invalidateQueries({ queryKey: queryKeys.specCatalog(projectId, targetKey) }),
    ];
    if (activeSelectedPath !== null) {
      invalidations.push(
        queryClient.invalidateQueries({ queryKey: queryKeys.specDocument(projectId, targetKey, activeSelectedPath) }),
      );
    }
    await Promise.all(invalidations);
  };
  const isRefreshing = catalogQuery.isFetching || documentQuery.isFetching;

  useEffect(() => {
    onRefreshingChange?.(isRefreshing);
  }, [isRefreshing, onRefreshingChange]);

  useImperativeHandle(ref, () => ({
    refresh,
    isRefreshing,
    clearSelection: () => setSelectedPath(null),
  }));

  const workspaceRootPath = taskId === undefined ? projectRootPath : workspaceQuery.data?.rootPath;

  return (
    <>
      <ResizablePanelGroup orientation="horizontal" className="min-h-0 flex-1">
        <ResizablePanel id="spec-content" minSize={280}>
          <div className="flex h-full min-w-0 flex-col">
            {activeSelectedPath !== null && (
              <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
                {workflowLabel && <span className="rounded border border-border bg-muted px-1.5 py-0.5 text-[10px] font-medium">{workflowLabel}</span>}
                <span className="min-w-0 flex-1 truncate font-mono text-xs">{activeSelectedPath}</span>
                <span className="text-[10px] text-muted-foreground">{activeSelectedPath.toLowerCase().endsWith(".mdx") ? "MDX" : "MD"}</span>
                {selectedDocument && <span className="text-[11px] text-muted-foreground">{selectedDocument.byteSize.toLocaleString()} B</span>}
                <Button size="icon-sm" variant={mode === "preview" ? "secondary" : "ghost"} aria-label={t("specs.preview")} onClick={() => setMode("preview")}><IconEye /></Button>
                <Button size="icon-sm" variant={mode === "source" ? "secondary" : "ghost"} aria-label={t("specs.source")} onClick={() => setMode("source")}><IconCode /></Button>
              </div>
            )}
            <SpecDocumentBody
              selectedPath={activeSelectedPath}
              content={documentQuery.data?.content}
              loading={documentQuery.isLoading}
              error={documentQuery.error ?? catalogQuery.error}
              mode={mode}
              markdownComponents={markdownComponents}
            />
          </div>
        </ResizablePanel>
        <ResizableHandle withHandle aria-label={t("specs.resizeTree")} />
        <ResizablePanel id="spec-tree" defaultSize={260} minSize={180} maxSize={480}>
          <div className="flex h-full min-h-0 flex-col border-l border-border">
            <div className="flex items-center gap-2 border-b border-border p-2">
              <div className="relative min-w-0 flex-1">
                <IconSearch className="pointer-events-none absolute left-3 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
                <Input className="h-8 pl-8 text-xs" value={filter} placeholder={t("specs.filter")} onChange={(event) => setFilter(event.target.value)} />
              </div>
              <Button type="button" size="sm" variant="outline" className="shrink-0" onClick={() => setSettingsOpen(true)}>
                {t("specs.manageSources")}
              </Button>
            </div>
            <div className="min-h-0 flex-1">
              {catalogQuery.isLoading ? <Status text={t("specs.loading")} />
                : catalogQuery.error !== null ? <Status text={localizeContractError(catalogQuery.error, t)} destructive />
                : documents.length === 0 ? (
                  <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
                    <p className="text-sm text-muted-foreground">{t("specs.empty")}</p>
                    <Button type="button" size="sm" variant="outline" onClick={() => setSettingsOpen(true)}>
                      {t("specs.manageSources")}
                    </Button>
                  </div>
                )
                  : <SpecTree documents={documents} selectedPath={activeSelectedPath} onSelect={setSelectedPath} />}
            </div>
            {catalogQuery.data?.truncated && <p className="border-t border-border px-3 py-2 text-[11px] text-amber-700">{t("specs.truncated")}</p>}
          </div>
        </ResizablePanel>
      </ResizablePanelGroup>
      <SpecSourceDialog
        open={settingsOpen}
        projectId={projectId}
        target={target}
        initialPath={workspaceRootPath}
        sources={catalogQuery.data?.sources ?? []}
        onOpenChange={setSettingsOpen}
      />
    </>
  );
});

function SpecDocumentBody({ selectedPath, content, loading, error, mode, markdownComponents }: {
  selectedPath: string | null;
  content: string | undefined;
  loading: boolean;
  error: unknown;
  mode: "preview" | "source";
  markdownComponents: Components;
}) {
  const { t } = useTranslation();
  if (selectedPath === null) return <Status text={t("specs.selectDocument")} />;
  if (loading) return <Status text={t("specs.loading")} />;
  if (error !== null) return <Status text={localizeContractError(error, t)} destructive />;
  if (content === undefined) return <Status text={t("specs.selectDocument")} />;
  if (mode === "source") {
    return <WorkspaceFileViewer content={content} path={selectedPath} target={null} />;
  }
  return (
    <ScrollArea className="min-h-0 flex-1">
      <article className="mx-auto max-w-4xl px-8 py-7">
        <MarkdownDocument content={content} components={markdownComponents} />
      </article>
    </ScrollArea>
  );
}

function Status({ text, destructive = false }: { text: string; destructive?: boolean }) {
  return <div data-selectable className={`flex h-full items-center justify-center p-6 text-sm ${destructive ? "text-destructive" : "text-muted-foreground"}`}>{text}</div>;
}
