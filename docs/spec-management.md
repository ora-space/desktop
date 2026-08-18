# Specification management

Ora exposes Markdown specifications as a read-only review surface for both a project root and a
task's authoritative workspace. The feature is intentionally independent from the OpenSpec
workflow-session store: workflow state tracks conversational steps, while Spec management indexes
documents already present on disk.

## Targets and automatic discovery

Every operation carries a tagged `SpecTarget`: either a project id or a task id. Task resolution uses
the same cwd as agent sessions, including linked worktrees and project-root tasks. Source discovery
is automatic and is never persisted as project configuration.

Ora recognizes these built-in source directories:

- OpenSpec: `openspec/specs`, `openspec/changes`
- Superpowers: `docs/superpowers/specs`, `docs/superpowers/plans`, `docs/plans`
- Custom: `specs`, `docs/specs`

Bounded discovery also recognizes Markdown and MDX below `spec`/`specs` directories and
workflow-owned `changes`/`plans` directories. General discovery honors Git ignore rules and excludes
generated directories. Built-in sources are scanned separately with ignore rules disabled so their
documents remain visible even when a repository ignores those directories. Exact duplicate paths
are merged using the host filesystem's case semantics, and overlapping documents belong to the
deepest detected source.

## API and security

The generated `spec` client namespace exposes catalog, read, and watch operations. Catalog and read
responses never expose absolute roots. On Desktop, spec watch uses a Tauri channel. The stream
completes when process shutdown begins so a mounted Specs view cannot block application exit; a
terminal error already queued at shutdown is emitted as `error` rather than a successful `end`.

All filesystem operations canonicalize the target root. Reads accept only `.md`/`.mdx` files that
still belong to the current automatically detected catalog, preventing traversal, symlink escape,
and stale-catalog authorization. Discovery uses Ora's injected bundled ripgrep with the existing
15-second, 8 MiB, and 10,000-result limits and reports truncation.

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
manual refresh, and mounted-only watching. Raw HTML and MDX JSX are not executed, local images are
blocked, and only catalog-member relative Markdown links navigate inside the panel.

## Using Spec management

1. Select a project to review its root checkout, or select a task to review that task's authoritative
   project-root or linked-worktree directory.
2. Open **Files**, then switch to **Specs** when reviewing specification documents.
3. Choose an entry from the workflow-grouped tree, filter by filename or path, and switch between
   rendered Markdown and line-numbered source when needed. Clicking **Specs** again clears the
   current selection.

Documents remain read-only; editing and deletion continue to belong to the user's normal filesystem
tools.
