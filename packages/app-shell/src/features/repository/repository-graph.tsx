import { useEffect, useMemo, useState, type ReactNode } from "react";
import type {
  ProjectBranch,
  RepositoryCommit,
  RepositoryCommitDetails,
  RepositorySnapshot,
  Task,
} from "@ora/contracts";
import { Badge, ScrollArea } from "@ora/ui";
import {
  IconGitBranch,
  IconGitCommit,
  IconGitFork,
  IconGitMerge,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { localizeContractError } from "../../i18n/contract-error";
import { useRepositoryCommit } from "../../state/hooks/use-repository-commit";
import { useRepositoryCommitDiff } from "../../state/hooks/use-repository-commit-diff";
import { RepositoryCommitDiffView } from "./repository-commit-diff-view";
import {
  buildRepositoryGraphLayout,
  REPOSITORY_GRAPH_LANE_COLORS,
  REPOSITORY_GRAPH_LANE_WIDTH,
  REPOSITORY_GRAPH_NODE_SIZE,
  REPOSITORY_GRAPH_ROW_HEIGHT,
  getRepositoryAuthorInitials,
  type RepositoryGraphLayout,
  type RepositoryGraphRowLayout,
} from "./repository-graph-layout";

interface RepositoryGraphProps {
  projectId: string;
  snapshot?: RepositorySnapshot;
  snapshotError: Error | null;
  branches: ProjectBranch[];
  projectTasks: Task[];
  selectedTask?: Task;
  activeBranch: string;
  loading: boolean;
}

/** Renders the bounded commit graph, ref rail, and selected commit detail pane. */
export function RepositoryGraph({
  projectId,
  snapshot,
  snapshotError,
  branches,
  projectTasks,
  selectedTask,
  activeBranch,
  loading,
}: RepositoryGraphProps) {
  const { t } = useTranslation();
  const [selectedCommitId, setSelectedCommitId] = useState<string | null>(null);
  const [detailCommitId, setDetailCommitId] = useState<string | null>(null);
  const [commitDiffPath, setCommitDiffPath] = useState<string | null>(null);
  const [commitDiffOpen, setCommitDiffOpen] = useState(false);
  const selectedCommit = snapshot?.commits.find((commit) => commit.id === selectedCommitId);
  const commitQuery = useRepositoryCommit(projectId, detailCommitId);
  const commitDiffQuery = useRepositoryCommitDiff(
    projectId,
    commitDiffOpen ? selectedCommit?.id ?? null : null,
    selectedCommit?.parents[0] ?? null,
    commitDiffPath,
    { enabled: commitDiffOpen },
  );
  const worktreeTasks = projectTasks.filter((task) => task.workspaceMode === "worktree");
  const graphLayout = useMemo(
    () => buildRepositoryGraphLayout(snapshot?.commits ?? []),
    [snapshot?.commits],
  );

  useEffect(() => {
    const nextCommitId = snapshot?.headCommitId ?? snapshot?.commits[0]?.id ?? null;
    setSelectedCommitId((current) => (
      current && snapshot?.commits.some((commit) => commit.id === current)
        ? current
        : nextCommitId
    ));
    setDetailCommitId((current) => (
      current && snapshot?.commits.some((commit) => commit.id === current)
        ? current
        : null
    ));
  }, [snapshot]);

  useEffect(() => {
    setCommitDiffOpen(false);
    setCommitDiffPath(null);
  }, [selectedCommitId]);

  const graphMessage = useMemo(() => {
    if (snapshotError) {
      return {
        title: t("repository.graphError"),
        description: localizeContractError(snapshotError, t),
      };
    }
    if (loading) {
      return {
        title: t("repository.loadingGraph"),
        description: t("repository.loadingGraphDescription"),
      };
    }
    if (!snapshot || snapshot.commits.length === 0) {
      return {
        title: t("repository.noCommits"),
        description: t("repository.noCommitsDescription"),
      };
    }
    return null;
  }, [loading, snapshot, snapshotError, t]);

  return (
    <div className="grid min-h-0 h-full grid-cols-[220px_minmax(0,1fr)_300px]">
      <RepositoryGraphRefRail
        snapshot={snapshot}
        branches={branches}
        worktreeTasks={worktreeTasks}
        loading={loading}
      />
      <section className="min-w-0 overflow-hidden border-r border-border">
        <div className="flex h-11 items-center gap-2 border-b border-border px-4">
          <IconGitCommit className="size-4 text-muted-foreground" />
          <span className="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">
            {t("repository.graph")}
          </span>
          <Badge variant="outline" className="ml-auto max-w-44 truncate font-mono text-[10px]">
            {snapshot?.currentBranch ?? activeBranch}
          </Badge>
          {snapshot && snapshot.workingTree.changedFiles > 0 && (
            <Badge variant="secondary" className="text-[10px]">
              {snapshot.workingTree.changedFiles}
            </Badge>
          )}
        </div>
        {graphMessage ? (
          <div className="flex h-[calc(100%-2.75rem)] items-center justify-center p-6">
            <div className="max-w-md text-center">
              <div className="mx-auto mb-4 flex size-12 items-center justify-center rounded-xl border border-border bg-muted/50">
                <IconGitFork className="size-6 text-sky-600" />
              </div>
              <h1 className="text-base font-semibold">{graphMessage.title}</h1>
              <p className="mt-2 text-sm leading-6 text-muted-foreground">
                {graphMessage.description}
              </p>
            </div>
          </div>
        ) : (
          <ScrollArea className="h-[calc(100%-2.75rem)]">
            <div className="relative">
              <RepositoryGraphConnections layout={graphLayout} />
              <div className="relative z-20 divide-y divide-border">
                {graphLayout.rows.map((row) => (
                <RepositoryCommitRow
                  key={row.commit.id}
                  row={row}
                  graphWidth={graphLayout.laneCount * REPOSITORY_GRAPH_LANE_WIDTH}
                  selected={row.commit.id === selectedCommitId}
                  onSelect={() => {
                    setSelectedCommitId(row.commit.id);
                    setDetailCommitId(row.commit.id);
                  }}
                />
                ))}
              </div>
            </div>
          </ScrollArea>
        )}
      </section>
      <RepositoryCommitDetailsPane
        commit={selectedCommit}
        detail={commitQuery.data}
        loading={detailCommitId !== null && commitQuery.isPending}
        error={detailCommitId === null ? null : commitQuery.error}
        selectedTask={selectedTask}
        activeBranch={activeBranch}
        onOpenDiff={(path) => {
          setCommitDiffPath(path);
          setCommitDiffOpen(true);
        }}
      />
      <RepositoryCommitDiffView
        commit={commitQuery.data}
        patch={commitDiffQuery.data}
        loading={commitDiffQuery.isPending}
        error={commitDiffQuery.error}
        initialPath={commitDiffPath}
        open={commitDiffOpen}
        onOpenChange={setCommitDiffOpen}
      />
    </div>
  );
}

interface RepositoryGraphRefRailProps {
  snapshot?: RepositorySnapshot;
  branches: ProjectBranch[];
  worktreeTasks: Task[];
  loading: boolean;
}

/** Groups repository refs and Ora-managed worktrees beside the commit list. */
function RepositoryGraphRefRail({
  snapshot,
  branches,
  worktreeTasks,
  loading,
}: RepositoryGraphRefRailProps) {
  const { t } = useTranslation();
  const localReferences = snapshot?.references.filter((reference) => reference.kind === "local") ?? [];
  const displayBranches = localReferences.length > 0
    ? localReferences.map((reference) => ({
      key: reference.name,
      label: reference.name,
      detail: reference.commitId.slice(0, 8),
    }))
    : branches.slice(0, 20).map((branch) => ({
      key: branch.refName,
      label: branch.displayName,
      detail: branch.refName,
    }));

  return (
    <aside className="min-h-0 overflow-hidden border-r border-border bg-sidebar/30">
      <ScrollArea className="h-full">
        <div className="space-y-5 p-3">
          <GraphRefGroup title={t("repository.localBranches")}>
            {loading && <GraphRefPlaceholder label={t("repository.loadingBranches")} />}
            {!loading && displayBranches.length === 0 && (
              <GraphRefPlaceholder label={t("repository.noBranches")} />
            )}
            {!loading && displayBranches.map((branch) => (
              <div
                key={branch.key}
                className="flex min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-xs hover:bg-muted/60"
              >
                <IconGitBranch className="size-3.5 shrink-0 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate font-medium">{branch.label}</span>
                <span className="max-w-20 truncate font-mono text-[10px] text-muted-foreground">
                  {branch.detail}
                </span>
              </div>
            ))}
          </GraphRefGroup>
          <GraphRefGroup title={t("repository.worktrees")}>
            {worktreeTasks.length === 0 && (
              <GraphRefPlaceholder label={t("repository.noWorktrees")} />
            )}
            {worktreeTasks.map((task) => (
              <div
                key={task.id}
                className="flex min-w-0 items-center gap-2 rounded-md px-2 py-1.5 text-xs hover:bg-muted/60"
              >
                <IconGitFork className="size-3.5 shrink-0 text-sky-600" />
                <span className="min-w-0 flex-1 truncate font-medium">{task.title}</span>
                <span className="max-w-20 truncate text-[10px] text-muted-foreground">
                  {t("repository.managedWorktree")}
                </span>
              </div>
            ))}
          </GraphRefGroup>
        </div>
      </ScrollArea>
    </aside>
  );
}

/** Renders one fixed-height commit row so the shared SVG topology has exact node coordinates. */
function RepositoryCommitRow({
  row,
  graphWidth,
  selected,
  onSelect,
}: {
  row: RepositoryGraphRowLayout;
  graphWidth: number;
  selected: boolean;
  onSelect: () => void;
}) {
  const { t } = useTranslation();
  const { commit } = row;
  const isMerge = commit.parents.length > 1;
  const className = selected
    ? "relative flex h-[72px] w-full min-w-0 gap-3 bg-accent/60 px-4 py-1.5 text-left transition-colors hover:bg-muted/50"
    : "relative flex h-[72px] w-full min-w-0 gap-3 px-4 py-1.5 text-left transition-colors hover:bg-muted/50";

  return (
    <button type="button" className={className} onClick={onSelect}>
      <GraphLane row={row} graphWidth={graphWidth} isMerge={isMerge} />
      <span className="min-w-0 flex-1">
        <span className="flex min-w-0 items-center gap-1.5">
          {commit.referenceNames.slice(0, 3).map((reference) => (
            <Badge key={reference} variant="secondary" className="max-w-32 truncate px-1.5 py-0 text-[10px]">
              {reference}
            </Badge>
          ))}
          {isMerge && (
            <span className="text-[10px] text-muted-foreground">
              {t("repository.mergeCommit")}
            </span>
          )}
        </span>
        <span className="mt-1 block truncate text-sm font-medium">{commit.subject}</span>
        <span className="mt-1 flex min-w-0 items-center gap-2 text-[11px] text-muted-foreground">
          <span className="truncate">{commit.authorName}</span>
          <span aria-hidden="true">·</span>
          <span>{formatCommitDate(commit.authoredAt)}</span>
          <span className="ml-auto font-mono">{commit.shortId}</span>
        </span>
      </span>
    </button>
  );
}

/** Draws every parent edge in one coordinate system so connections stay continuous between rows. */
function RepositoryGraphConnections({ layout }: { layout: RepositoryGraphLayout }) {
  const graphWidth = layout.laneCount * REPOSITORY_GRAPH_LANE_WIDTH;
  const graphHeight = layout.rows.length * REPOSITORY_GRAPH_ROW_HEIGHT;

  return (
    <svg
      aria-hidden="true"
      className="pointer-events-none absolute top-0 left-4 z-10 overflow-visible"
      width={graphWidth}
      height={graphHeight}
      viewBox={[0, 0, graphWidth, graphHeight].join(" ")}
    >
      {layout.rows.flatMap((row, rowIndex) => row.parents.map((parent) => {
        const targetRowIndex = parent.parentRowIndex;
        const targetLaneIndex = targetRowIndex === null
          ? parent.laneIndex
          : layout.rows[targetRowIndex].laneIndex;
        const fromX = row.laneIndex * REPOSITORY_GRAPH_LANE_WIDTH + REPOSITORY_GRAPH_LANE_WIDTH / 2;
        const toX = targetLaneIndex * REPOSITORY_GRAPH_LANE_WIDTH + REPOSITORY_GRAPH_LANE_WIDTH / 2;
        const fromY = rowIndex * REPOSITORY_GRAPH_ROW_HEIGHT + REPOSITORY_GRAPH_ROW_HEIGHT / 2;
        const toY = targetRowIndex === null
          ? graphHeight
          : targetRowIndex * REPOSITORY_GRAPH_ROW_HEIGHT + REPOSITORY_GRAPH_ROW_HEIGHT / 2;

        return (
          <path
            key={[row.commit.id, parent.parentId].join("-")}
            d={buildGraphConnectionPath(fromX, fromY, toX, toY)}
            fill="none"
            stroke={REPOSITORY_GRAPH_LANE_COLORS[parent.colorIndex % REPOSITORY_GRAPH_LANE_COLORS.length]}
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeOpacity="0.94"
            strokeWidth="2.25"
            vectorEffect="non-scaling-stroke"
          />
        );
      }))}
    </svg>
  );
}

/** Uses a short rounded turn followed by a straight lane segment like native Git graph clients. */
function buildGraphConnectionPath(fromX: number, fromY: number, toX: number, toY: number): string {
  if (fromX === toX) {
    return `M ${fromX} ${fromY} V ${toY}`;
  }

  return [
    `M ${fromX} ${fromY}`,
    `C ${fromX} ${fromY + 10}, ${toX} ${fromY + 10}, ${toX} ${fromY + 18}`,
    `V ${toY}`,
  ].join(" ");
}

/** Renders the node marker while the shared SVG layer owns all inter-row geometry. */
function GraphLane({
  row,
  graphWidth,
  isMerge,
}: {
  row: RepositoryGraphRowLayout;
  graphWidth: number;
  isMerge: boolean;
}) {
  const laneColor = REPOSITORY_GRAPH_LANE_COLORS[
    row.laneColorIndex % REPOSITORY_GRAPH_LANE_COLORS.length
  ];
  const authorInitials = getRepositoryAuthorInitials(row.commit.authorName, row.commit.authorEmail);

  return (
    <span className="relative z-20 h-full shrink-0" style={{ width: graphWidth }}>
      <span
        className="absolute top-1/2 left-0 flex size-[18px] -translate-y-1/2 items-center justify-center rounded-full border-2 bg-background text-[8px] font-bold leading-none ring-1 ring-background"
        title={`${row.commit.authorName} <${row.commit.authorEmail}>`}
        style={{
          borderColor: laneColor,
          backgroundColor: "var(--background)",
          left: row.laneIndex * REPOSITORY_GRAPH_LANE_WIDTH
            + (REPOSITORY_GRAPH_LANE_WIDTH - REPOSITORY_GRAPH_NODE_SIZE) / 2,
        }}
      >
        {isMerge
          ? <IconGitMerge className="size-2.5" style={{ color: laneColor }} />
          : <span style={{ color: laneColor }}>{authorInitials}</span>}
      </span>
    </span>
  );
}

interface RepositoryCommitDetailsPaneProps {
  commit?: RepositoryCommit;
  detail?: RepositoryCommitDetails;
  loading: boolean;
  error: Error | null;
  selectedTask?: Task;
  activeBranch: string;
  onOpenDiff: (path: string) => void;
}

/** Shows selected commit metadata and changed paths without leaving the repository workspace. */
function RepositoryCommitDetailsPane({
  commit,
  detail,
  loading,
  error,
  selectedTask,
  activeBranch,
  onOpenDiff,
}: RepositoryCommitDetailsPaneProps) {
  const { t } = useTranslation();

  return (
    <aside className="min-w-0 bg-muted/20">
      <div className="border-b border-border px-4 py-3">
        <p className="text-xs font-semibold uppercase tracking-[0.12em] text-muted-foreground">
          {t("repository.selection")}
        </p>
      </div>
      <ScrollArea className="h-[calc(100%-2.75rem)]">
        <div className="space-y-4 p-4">
          {selectedTask && (
            <div className="rounded-md border border-border bg-background p-3">
              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                <IconGitBranch className="size-3.5" />
                {t("repository.selectedWorktree")}
              </div>
              <p className="mt-1 truncate text-sm font-medium">{selectedTask.title}</p>
              <p className="mt-1 truncate font-mono text-xs text-muted-foreground">{activeBranch}</p>
            </div>
          )}
          {!commit && (
            <p className="text-sm leading-6 text-muted-foreground">
              {t("repository.selectCommit")}
            </p>
          )}
          {commit && (
            <section className="space-y-3">
              <div>
                <p className="text-sm font-semibold leading-5">{detail?.subject ?? commit.subject}</p>
                <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">
                  {detail?.id ?? commit.id}
                </p>
              </div>
              <div className="grid grid-cols-2 gap-2 text-xs">
                <DetailMetric label={t("repository.author")} value={detail?.authorName ?? commit.authorName} />
                <DetailMetric
                  label={t("repository.committed")}
                  value={formatCommitDate(detail?.authoredAt ?? commit.authoredAt)}
                />
                <DetailMetric label={t("repository.parents")} value={String(detail?.parents.length ?? commit.parents.length)} />
                <DetailMetric
                  label={t("repository.filesChanged")}
                  value={detail ? String(detail.files.length) : "—"}
                />
              </div>
              {loading && (
                <p className="text-xs text-muted-foreground">{t("repository.loadingCommit")}</p>
              )}
              {error && (
                <p className="text-xs text-destructive">{localizeContractError(error, t)}</p>
              )}
              {!loading && !error && detail && (
                <div className="overflow-hidden rounded-md border border-border bg-background">
                  <div className="border-b border-border px-3 py-2 text-xs font-semibold">
                    {t("repository.changedFiles")}
                  </div>
                  {detail.files.length === 0 && (
                    <p className="px-3 py-3 text-xs text-muted-foreground">
                      {t("repository.noChangedFiles")}
                    </p>
                  )}
                  {detail.files.map((file) => (
                    <button
                      type="button"
                      key={[file.status, file.path].join("-")}
                      className="flex w-full min-w-0 items-center gap-2 border-b border-border px-3 py-2 text-left last:border-b-0 hover:bg-muted/50"
                      aria-label={`${t("repository.openDiff")}: ${file.path}`}
                      onClick={() => onOpenDiff(file.path)}
                    >
                      <Badge variant="outline" className="w-10 justify-center px-1 font-mono text-[10px]">
                        {file.status}
                      </Badge>
                      <span className="min-w-0 truncate font-mono text-[11px]">{file.path}</span>
                    </button>
                  ))}
                </div>
              )}
            </section>
          )}
        </div>
      </ScrollArea>
    </aside>
  );
}

/** Renders one compact label/value pair in the commit detail pane. */
function DetailMetric({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-md border border-border bg-background p-2">
      <p className="text-[10px] uppercase tracking-wide text-muted-foreground">{label}</p>
      <p className="mt-1 truncate text-xs font-medium">{value}</p>
    </div>
  );
}

/** Formats the ISO timestamp returned by Git into a compact local display label. */
function formatCommitDate(authoredAt: string): string {
  if (!authoredAt) {
    return "—";
  }

  const date = new Date(authoredAt);
  return Number.isNaN(date.getTime())
    ? authoredAt.slice(0, 10)
    : new Intl.DateTimeFormat(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
    }).format(date);
}

/** Groups one ref family in the graph navigation rail. */
function GraphRefGroup({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section>
      <div className="mb-1 flex items-center gap-1.5 px-2 text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
        <IconGitBranch className="size-3" />
        <span>{title}</span>
      </div>
      <div className="space-y-0.5">{children}</div>
    </section>
  );
}

/** Shows graph rail loading and empty states without shifting the surrounding layout. */
function GraphRefPlaceholder({ label }: { label: string }) {
  return <p className="px-2 py-1 text-xs text-muted-foreground">{label}</p>;
}
