# Specification management

Ora exposes Markdown specifications as a read-only review surface for both a project root and a
task's authoritative workspace. The feature is intentionally independent from the OpenSpec
workflow-session store: workflow state tracks conversational steps, while Spec management indexes
documents already present on disk.

## Targets and configuration

Every operation carries a tagged `SpecTarget`: either a project id or a task id. Task resolution uses
the same cwd as agent sessions, including linked worktrees and project-root tasks. Source overrides
are persisted per project and therefore apply consistently to the main checkout and every worktree.

Ora considers these default candidates:

- OpenSpec: `openspec/specs`, `openspec/changes`
- Superpowers: `docs/superpowers/specs`, `docs/superpowers/plans`, `docs/plans`
- Custom: `specs`, `docs/specs`

Bounded discovery finds Markdown and MDX below controlled `spec`/`specs` directories and workflow
owned `changes`/`plans` directories. It honors Git ignore rules and excludes generated directories.
Explicit enabled sources are enumerated separately with ignore rules disabled. Exact duplicate paths
are merged using the host filesystem's case semantics, and overlapping documents belong to the
deepest enabled source.

The SQLite `project_spec_source_overrides` table stores normalized relative paths, workflow,
visibility, audit timestamps, and soft-delete state. Replacement is atomic. Project aggregate
deletion soft-deletes these rows in the same transaction.

## API and security

The generated `spec` client namespace exposes catalog, read, source resolution, project-source
replacement, and watch operations. Catalog and read never expose absolute roots. The existing
`task.getWorkspace` operation supplies the optional branch and absolute root needed only for the
platform directory picker.

All filesystem operations canonicalize target and selected paths. Reads accept only `.md`/`.mdx`
files that still belong to the current effective catalog, preventing traversal, symlink escape, and
stale-source authorization. Discovery uses Ora's injected bundled ripgrep with the existing 15-second,
8 MiB, and 10,000-result limits and reports truncation.

## Frontend behavior

`WorkspaceReviewLayout` owns the established 900 px resizable right panel and expanded overlay.
Project context offers **Files** only; task context offers **Changes** and **Files**. Spec documents
live inside the Files panel as a dedicated **Specs** sub-view alongside **Explorer** and **Search**.
Task Files opens on Explorer by default; project Files opens on Specs by default because project-root
review does not expose a worktree file explorer.

The Specs sub-view places read-only content on the left and the grouped source tree on the right. It
starts without an automatic document selection; the viewer stays empty until the user picks a tree
entry, and clicking Specs again while already on Specs clears the current selection. It supports a
200 ms filename/path filter, safe GFM preview, the existing line-numbered Shiki source viewer,
manual refresh, and mounted-only watching. Raw HTML and MDX JSX are not executed, local
images are blocked, and only catalog-member relative Markdown links navigate inside the panel.

Spec source configuration is available only while the Specs sub-view is active: the **Manage sources**
action in the tree sidebar (including the empty state) opens the project-wide source dialog.

Source configuration uses `PlatformAdapter.selectPath({ kind: "directory" })`, preserving the Web
directory-tree picker and Desktop native picker. The dialog shows a single description line plus a
compact workspace context bar with the current root path (truncated inline, full path on hover).
Picker failures return
stable public codes such as `spec_source_outside_workspace`, `spec_source_workspace_root`, and the
existing `file_system_path_*` codes; the frontend renders them through the shared contract error
localizer.

## Using Spec management

1. Select a project to review its root checkout, or select a task to review that task's authoritative
   project-root/linked-worktree directory.
2. Open **Files** from the review controls, then switch to the **Specs** sub-view when reviewing
   specification documents. Specs opens with an empty viewer and does not restore or auto-open a
   previous document; choose an entry from the workflow-grouped tree, filter by filename/path, and
   switch between rendered Markdown and line-numbered source when needed. Clicking **Specs** again
   while already on that sub-view clears the current selection.
3. Open the source dialog from **Manage sources** to enable or disable default or
   discovered sources. Use **Add directory** for an arbitrary project-relative source, then choose
   OpenSpec, Superpowers, or a named custom workflow.
4. Save once to atomically apply the source set to the project and all of its worktrees. Missing
   sources remain visible in this dialog so the configuration survives branch differences.

The project root itself cannot be registered as a source: a source must be a contained subdirectory,
which prevents unrelated repository Markdown from being reclassified as specifications. Documents
remain read-only; editing and deletion continue to belong to the user's normal filesystem tools.
