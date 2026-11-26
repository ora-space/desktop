# Workflow Packages

English | [中文](workflow-packages.zh.md)

A `kind = "workflow"` plugin package delivers workflow documents. Importing one installs the plugin
and turns every document it carries into a workflow with one published snapshot, so a user receives
a set of runnable workflows in one step. The imported workflows are user data: uninstalling the
plugin never removes them. The decision is the
[workflow package format](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/category/workflow/0-workflow-package-format.md)
ADR.

## Package layout

```text
my-workflows/
├── orax.toml            kind = "workflow"; no kind-specific section
├── assets/workflows/
│   ├── 1.0.0.json
│   └── release.reactflow.json
└── logo.svg             optional
```

- `assets/workflows/` is required and holds 1 to 256 `*.json` documents. Only regular `.json` files
  directly in that directory count; a README, a subdirectory or a symlink is ignored rather than
  rejected.
- The package is processless: no `main.js`, no Deno runtime, no kind-specific manifest section;
  `[workbench]` and `[webview]` are refused as on every kind that does not define them.

## Document format

A document is the editor's exported workflow file, stored verbatim: export a workflow from the
library and drop the file into `assets/workflows/`.

```json
{
  "id": "d90c3588-1238-444d-afb4-908091277789",
  "name": "发布流程",
  "version": "1.0.0",
  "viewport": { "x": 0, "y": 0, "zoom": 1 },
  "nodes": [
    {
      "id": "start",
      "type": "workflow",
      "position": { "x": 120, "y": 260 },
      "data": { "kind": "start" }
    }
  ],
  "edges": [],
  "annotations": [],
  "globalVariables": []
}
```

- `name` is required and becomes the workflow name. Names must be unique, so a document whose name
  already exists is refused rather than suffixed.
- `version` is optional (see below).
- Everything else, including React Flow runtime fields and the export's own `id` and `updatedAt`, is
  stored in the snapshot's `graph` without interpretation; the editor drops runtime fields on the
  first save. This is what makes an exported file round-trip unchanged.
- Import does not screen node kinds. The editor drops kinds it cannot render when it loads the
  workflow (see [Workflows](../workflows/workflows.md#graph-storage)), so write documents against the
  node catalog of the Ora version you target.

## Import

Workflow packages are imported through the ordinary local archive import (`importPlugin`) and through
marketplace installation and upgrade; the package's `kind` decides what happens.

1. The archive is verified, extracted and committed under the `local` namespace. A package that fails
   to install creates no workflows.
2. Each document is then read and imported on its own: parse the JSON, take `name` and the optional
   `version`, validate the graph with the run engine's parser, create the workflow, publish a
   snapshot and activate it.
3. The response reports one outcome per document (`imported` with the workflow id, name and version,
   or `failed` with a reason).

Because validation reuses the run engine's parser, an imported workflow is held to exactly the rules
a run would apply; a document that could never execute is refused at import. A broken document costs
only itself, and each document either becomes a workflow with one published snapshot or does not
become one at all.

### Marketplace install and upgrade

A marketplace install of a Workflow package imports the documents it carries, so the package is
usable as soon as it lands; a pack imports what each newly installed workflow member carries (members
already installed before the pack run are skipped and do not enter a new install cycle). The copy
that results is the user's data, so the marketplace keeps a durable record per source path
(`(source namespace, plugin identifier, document path)`) and evaluates each path against its current
record, copy liveness, and the operation (`install`, `update`, or card `retry`), as specified in the
[conflict recovery and reinstall ADR](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/category/workflow/20261124-marketplace-workflow-conflict-recovery.md):

- **Surviving copies stay untouched.** A path whose `imported` record still points to a live copy
  (`is_deleted = 0`) returns `skipped_processed` on every later install, upgrade, and retry—even if
  the user edited or renamed that copy or uninstalled and reinstalled the package.
- **Name conflicts stay retryable.** Names are checked case-insensitively (`COLLATE NOCASE`). When a
  document collides with a surviving workflow, the user's workflow keeps its name and the source
  path records `conflict_skipped`, returning `skipped_conflict` with the blocking workflow's name and
  whether that conflict was already recorded (`knownConflict`). Every later install, upgrade, and
  card retry re-checks the library; once the user renames or deletes the blocking workflow, the next
  install, upgrade, or retry imports the document.
- **Install cycles govern deleted copies.** A successful marketplace install (including reinstalling
  an uninstalled package) opens a new install cycle for that plugin—or, for a pack, only for the
  workflow members this run actually installed. Reinstalling reconsiders any `imported` record whose
  copy was soft-deleted: it re-imports the deleted copy without duplicating surviving ones, records
  `conflict_skipped` if the name is now taken, and if recovery fails for another reason
  conditionally clears the deleted-copy record back to unrecorded so the card retry can try again
  (falling back to "fix and reinstall" only if clearing the record itself fails). Upgrades and card
  retries stay inside the current cycle: they respect in-cycle deletions and return
  `skipped_deleted_copy` with "uninstall and reinstall to restore".
- **Card retry.** The installed Workflow plugin card offers "Retry importing the missing documents"
  (a pack's workflow members are retried from each member's own installed card). It re-evaluates
  unrecorded paths and `conflict_skipped` paths without reinstalling, while leaving surviving copies
  untouched and copies deleted in the current cycle deleted.
- **Conditional atomic writes and reporting.** Source transitions are conditional on the state that
  was read, and copy creation commits with its draft, initial published snapshot, and `imported`
  record in one transaction. A concurrent name collision during commit rolls back and reports
  `skipped_conflict` rather than `failed`. Installs (including packs, naming each member and linking
  to its installed card) and card retries open the import report whenever any conflict occurs;
  upgrades open the report for first-time conflicts (listing sibling known conflicts as "still
  unresolved") and keep known conflicts and deleted copies as neutral summary counts—even when the
  upgrade report opens for another reason, deleted copies stay out of the "not imported" list. When
  nothing is imported and there is a failure, a first conflict, or an install/retry conflict, the
  toast stops using plain success styling. "Not imported" entries do not participate in MCP / Skill
  dependency checks.

Local `.orax` import is unchanged: it imports every document it carries and reports a name conflict
as that document's failure.

### Uninstalling a workflow plugin

The uninstall dialog lists the surviving workflows imported from that marketplace plugin. For a
Pack it lists copies from members being removed. The library and the create-run menu mark those
copies “Source plugin uninstalled”. The mark allows editing and running a copy. Running it still
requires its MCP and Skill dependencies, which a Pack uninstall may also remove. Reinstalling the same plugin
ID clears the mark. Local `.orax` imports also survive uninstall. They have no marketplace source
record, so they do not appear in the retained-copy list or receive a source mark.

### Plugin readiness

A document records the MCP plugins and Skills its Agent nodes bind, so importing a package can leave
workflows that reference plugins missing from this machine. The import therefore reports plugin
readiness next to the per-document outcomes: for a package that carried workflow documents, the
plugin manager opens an import result that reads each created workflow's graph back and lists every
MCP and Skill it references with its state (`installed`, `unavailable`, `missing`), the Agent nodes
that use it, and the same install, configure, or review entry the editor's own import preview
offers. A document the import refused is listed with its package path and the backend's reason.

The check is presentation only: it never blocks the import or installs anything on its own, and a
missing reference simply fails the run at the node that uses it. Both import entries resolve
dependencies with the same rules, so the same package reports the same readiness whether it is
imported from the plugin manager or from the editor's workflow import.

A workflow whose graph cannot be read back is reported as unreadable instead of as needing nothing;
while one of those is outstanding the result states no package-wide readiness verdict and covers
only what it could read. Import counts, workflow names, and refused documents remain visible while
dependency reads are pending or fail; a catalog failure only prevents the dependency verdict.
When every document was refused, the result lists just the refusals without querying dependency
catalogs. The review action for an invalid MCP targets its plugin id in the manager's search, so an
earlier search cannot hide the plugin's diagnostic.

## Version

The published version is the first usable candidate from: the document's `version`, the file name
without `.reactflow.json` or `.json`, the workflow title, and finally an automatic
`v{timestamp_millis}`. A candidate is skipped, not sanitized, when it is blank, longer than 128 bytes,
`.`, `..`, `draft`, or contains a path separator or control character. Renaming a file therefore does
not change the version of a document that declares one.

Bare `.json` workflow files are imported by the workflow editor's own import, not here, and nothing
writes workflows back into a package.
