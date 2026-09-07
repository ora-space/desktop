import { useMemo, useState } from "react";
import type { FileData } from "react-diff-view";
import {
  IconChevronDown,
  IconChevronRight,
  IconFile,
  IconFileCode,
  IconFileTypeCss,
  IconFileTypeHtml,
  IconFileTypeJs,
  IconFileTypeJsx,
  IconFileTypeRs,
  IconFileTypeTs,
  IconFileTypeTsx,
  IconFilter,
  IconFolder,
  IconFolderOpen,
  IconJson,
  IconLayoutSidebarRightCollapse,
  IconMarkdown,
  IconX,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import {
  buildDiffFileTree,
  filterDiffFiles,
  type DiffFileTreeNode,
} from "./task-diff-file-tree-utils";

interface TaskDiffFileTreeProps {
  files: FileData[];
  selectedPath: string;
  onSelect: (path: string) => void;
  /** Collapses the whole tree; the button sits at the right end of the header. */
  onCollapse?: () => void;
}

/** Renders the changed files as a compact, collapsible IDE-style directory tree. */
export function TaskDiffFileTree({
  files,
  selectedPath,
  onSelect,
  onCollapse,
}: TaskDiffFileTreeProps) {
  const { t } = useTranslation();
  const [filterOpen, setFilterOpen] = useState(false);
  const [filter, setFilter] = useState("");
  const visibleFiles = useMemo(
    () => filterDiffFiles(files, filter),
    [files, filter],
  );
  const nodes = buildDiffFileTree(visibleFiles);

  return (
    <aside
      className="flex h-full min-w-0 flex-col overflow-hidden border-l border-border bg-muted/10"
      aria-label={t("diff.fileTree")}
    >
      <div className="flex h-10 shrink-0 items-center border-b border-border px-3">
        <span className="text-[11px] font-semibold uppercase tracking-[0.06em] text-muted-foreground">
          {t("diff.files")}
        </span>
        <span className="ml-auto text-[10px] tabular-nums text-muted-foreground">
          {visibleFiles.length === files.length
            ? files.length
            : `${visibleFiles.length}/${files.length}`}
        </span>
        <button
          type="button"
          className={`ml-1.5 flex size-7 items-center justify-center rounded-md outline-none transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring ${
            filterOpen ? "bg-muted text-foreground" : "text-muted-foreground"
          }`}
          aria-label={t("diff.filterFiles")}
          aria-pressed={filterOpen}
          title={t("diff.filterFiles")}
          onClick={() => setFilterOpen((open) => !open)}
        >
          <IconFilter className="size-3.5" />
        </button>
        {onCollapse !== undefined && (
          <button
            type="button"
            className="ml-1.5 flex size-7 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            aria-label={t("diff.toggleFileTree")}
            title={t("diff.toggleFileTree")}
            onClick={onCollapse}
          >
            <IconLayoutSidebarRightCollapse className="size-3.5" />
          </button>
        )}
      </div>
      {filterOpen && (
        <div className="relative shrink-0 border-b border-border p-2">
          <IconFilter className="pointer-events-none absolute left-4 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <input
            autoFocus
            value={filter}
            onChange={(event) => setFilter(event.target.value)}
            className="h-8 w-full rounded-md border border-input bg-background pl-8 pr-8 text-xs outline-none placeholder:text-muted-foreground focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/30"
            aria-label={t("diff.filterFiles")}
            placeholder={t("diff.filterFilesPlaceholder")}
          />
          {filter !== "" && (
            <button
              type="button"
              className="absolute right-3 top-1/2 flex size-6 -translate-y-1/2 items-center justify-center rounded text-muted-foreground hover:bg-muted hover:text-foreground"
              aria-label={t("diff.clearFileFilter")}
              onClick={() => setFilter("")}
            >
              <IconX className="size-3.5" />
            </button>
          )}
        </div>
      )}
      <nav
        className="ora-scroll-region min-h-0 flex-1 overflow-y-auto overflow-x-hidden py-1"
        aria-label={t("diff.changedFilesNavigation")}
      >
        {nodes.length === 0 ? (
          <p className="px-3 py-6 text-center text-xs text-muted-foreground">
            {t("diff.noMatchingFiles")}
          </p>
        ) : (
          nodes.map((node) => (
            <DiffTreeNode
              key={`${node.kind}:${node.path}`}
              node={node}
              depth={0}
              selectedPath={selectedPath}
              onSelect={onSelect}
            />
          ))
        )}
      </nav>
    </aside>
  );
}

interface DiffTreeNodeProps {
  node: DiffFileTreeNode;
  depth: number;
  selectedPath: string;
  onSelect: (path: string) => void;
}

/** Displays one directory branch or selectable changed-file leaf. */
function DiffTreeNode({
  node,
  depth,
  selectedPath,
  onSelect,
}: DiffTreeNodeProps) {
  const [expanded, setExpanded] = useState(true);
  if (node.kind === "directory") {
    return (
      <div>
        <button
          type="button"
          className="relative flex h-8 w-full items-center gap-1.5 truncate text-left text-xs text-muted-foreground outline-none hover:bg-muted/60 hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
          style={{ paddingLeft: 8 + depth * 14 }}
          aria-expanded={expanded}
          onClick={() => setExpanded((current) => !current)}
        >
          {expanded ? (
            <IconChevronDown className="size-3 shrink-0" />
          ) : (
            <IconChevronRight className="size-3 shrink-0" />
          )}
          {expanded ? (
            <IconFolderOpen className="size-3.5 shrink-0 text-amber-600/80" />
          ) : (
            <IconFolder className="size-3.5 shrink-0 text-amber-600/80" />
          )}
          <span className="truncate" title={node.path}>
            {node.name}
          </span>
        </button>
        {expanded && (
          <div className="relative">
            <span
              aria-hidden="true"
              className="pointer-events-none absolute inset-y-0 w-px bg-border/80"
              style={{ left: 14 + depth * 14 }}
            />
            {node.children.map((child) => (
              <DiffTreeNode
                key={`${child.kind}:${child.path}`}
                node={child}
                depth={depth + 1}
                selectedPath={selectedPath}
                onSelect={onSelect}
              />
            ))}
          </div>
        )}
      </div>
    );
  }

  const selected = selectedPath === node.path;
  return (
    <button
      type="button"
      className={`relative flex h-8 w-full items-center gap-1.5 truncate text-left text-xs outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring ${
        selected
          ? "bg-accent text-accent-foreground"
          : "text-foreground/85 hover:bg-muted/60"
      }`}
      style={{ paddingLeft: 23 + depth * 14 }}
      aria-current={selected ? "page" : undefined}
      title={node.path}
      onClick={() => onSelect(node.path)}
    >
      <DiffFileIcon path={node.path} />
      <span className="min-w-0 flex-1 truncate">{node.name}</span>
    </button>
  );
}

/** Uses familiar language and document glyphs while retaining a generic code fallback. */
function DiffFileIcon({ path }: { path: string }) {
  const lastDot = path.lastIndexOf(".");
  const extension =
    lastDot === -1 ? "" : path.slice(lastDot).toLocaleLowerCase();
  const className = "size-3.5 shrink-0";
  if (extension === ".ts")
    return <IconFileTypeTs className={`${className} text-sky-600`} />;
  if (extension === ".tsx")
    return <IconFileTypeTsx className={`${className} text-sky-600`} />;
  if (extension === ".js")
    return <IconFileTypeJs className={`${className} text-amber-600`} />;
  if (extension === ".jsx")
    return <IconFileTypeJsx className={`${className} text-amber-600`} />;
  if (extension === ".rs")
    return <IconFileTypeRs className={`${className} text-orange-600`} />;
  if (extension === ".css")
    return <IconFileTypeCss className={`${className} text-violet-600`} />;
  if (extension === ".html")
    return <IconFileTypeHtml className={`${className} text-orange-600`} />;
  if (extension === ".json")
    return <IconJson className={`${className} text-amber-600`} />;
  if (extension === ".md" || extension === ".mdx") {
    return <IconMarkdown className={`${className} text-sky-700`} />;
  }
  if (extension === "")
    return <IconFile className={`${className} text-muted-foreground`} />;
  return <IconFileCode className={`${className} text-muted-foreground`} />;
}
