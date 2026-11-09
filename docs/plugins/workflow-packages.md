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
usable as soon as it lands; a pack imports what each newly installed workflow member carries. The
copy that results is the user's data, so the marketplace keeps a durable record per source path
(`(source namespace, plugin identifier, document path)`) and only imports paths without a terminal
record:

- A path that already has a copy is skipped on every later install, upgrade and retry. The copy is
  never overwritten, renamed, or removed, even when the new version changes that document.
- A document whose name is already taken keeps the user's workflow and is reported as a conflict
  skip once; later upgrades report it as an already handled path instead of failing again.
- Any other failure leaves no record, so it can be retried. `InstallPluginResponse` and
  `UpdatePluginResponse` report these outcomes per document, distinguishing imported, already
  handled, first name-conflict skip, and failed, and naming the member plugin that carried each
  document.
- The installed Workflow package's card offers "retry failed document imports". It reconsiders only
  paths without a terminal record, so a successful or conflict-skipped path stays as it is, and an
  upgrade retries those same paths on its way through.
- The records outlive the copy: uninstalling and reinstalling the package, or deleting a workflow,
  does not make a path look new. A workflow the user deleted therefore stays deleted.

Local `.orax` import is unchanged: it imports every document it carries and reports a name conflict
as that document's failure.

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
