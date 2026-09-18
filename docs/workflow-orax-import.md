# Workflow orax Import

English | [中文](workflow-orax-import.zh.md)

A `kind = "workflow"` plugin package is a delivery vehicle for workflow documents. Importing one
installs the plugin **and** turns every document it carries into a workflow with one published
snapshot, so a user can receive a set of runnable workflows in a single step instead of exporting
and re-importing each one.

The imported workflows are user data. They outlive the package that carried them: uninstalling the
plugin never removes them.

## Package layout

```
my-workflows/
├── orax.toml            # kind = "workflow"; no kind-specific section
├── assets/workflows/
│   ├── 1.0.0.json
│   └── release.reactflow.json
└── logo.svg             # optional
```

- `assets/workflows/` is required and must hold at least one `*.json` document, at most 256.
- Only regular `*.json` files **directly** under that directory are workflows. A README, a
  subdirectory, or a symlink is ignored rather than rejected, because the contract is "every
  document in this directory is imported" and a stray file is no reason to refuse an otherwise
  valid package.
- The package is processless: it ships no `main.js`, the host starts no Deno runtime for it, and
  `orax.toml` carries no kind-specific section. `[workbench]` and `[webview]` are refused on this
  kind exactly as they are on every other kind that does not define them.

The manifest rules themselves are the ones [plugin packaging](../specs/plugin-packaging.md)
already states for every kind; this document covers only what `workflow` adds.

## Document format

A workflow document is the editor's exported workflow file, stored verbatim. Export a workflow
from the workflow library and drop the file into `assets/workflows/`; nothing has to be rewritten
in between.

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

- `name` is required and becomes the workflow's name. Names must stay unique, so a document whose
  name an existing workflow already uses is refused rather than silently suffixed.
- `version` is optional; see below.
- Everything else — including the editor's React Flow runtime fields (`measured`, `selected`,
  `dragging`, `zIndex`) and the export's own `id` and `updatedAt` — is stored as part of the
  snapshot's `graph` without being interpreted. The `graph` column is an opaque string to the
  definition layer, the run engine ignores every field but `nodes` and `edges`, and the editor
  drops the runtime fields the first time the workflow is saved. Keeping the document intact is
  what makes an exported file round-trip unchanged.

## Import flow

Importing goes through the same operation as any other local `.orax` archive
(`client.plugin.import`); there is no separate workflow-import entry point. A package's behavior
is decided by its `kind`, not by which button opened the file chooser.

1. The archive is verified, extracted, and committed under the reserved `local` namespace. A
   package that fails to install creates no workflows at all.
2. Each document is read and imported **on its own**.
3. Per document: parse the JSON, take `name` and the optional `version`, validate the graph
   through the run engine's parser, create the workflow, then publish a snapshot and activate it.
4. The response reports one outcome per document:

```json
{
  "pluginId": "local/my-workflows",
  "outcome": { "state": "installed" },
  "workflows": [
    {
      "state": "imported",
      "sourceFile": "assets/workflows/1.0.0.json",
      "workflowId": "…",
      "name": "发布流程",
      "version": "1.0.0"
    },
    {
      "state": "failed",
      "sourceFile": "assets/workflows/2.0.0.json",
      "reason": "document is not valid JSON: …"
    }
  ]
}
```

Graph validation reuses the run engine's parser rather than a second implementation, so an
imported workflow is held to exactly the rules a run would hold it to. A document whose graph
could never execute is refused at import instead of being stored and discovered when a run starts.

## Version derivation

The published version is chosen in this order:

| Order | Source                                            | Example                               |
| ----- | ------------------------------------------------- | ------------------------------------- |
| 1     | The document's own `version` field                | `"1.0.0"` from `"version": "1.0.0"`   |
| 2     | The file name, minus `.reactflow.json` or `.json` | `1.0.0` from `1.0.0.json`             |
| 3     | The workflow title                                | `发布流程`                            |
| 4     | Automatic `v{timestamp_millis}`                   | when none of the above is publishable |

A candidate is refused rather than sanitized when it is blank, longer than 128 bytes, `.` or `..`,
`draft`, or contains a path separator or a control character. Sanitizing would publish under a name
its author never wrote; falling through instead lets the next source apply, and an unusable
document still gets a valid automatic version.

Because the file name is only a fallback, renaming a packaged document does not change the version
as long as the document declares one — which is why the packaging tool writes both.

## Failure semantics

- **A broken document costs only itself.** Every document is imported independently, so one
  malformed or unrunnable file never prevents the working workflows beside it from landing.
- **A bad package costs nothing but itself.** If the archive cannot be installed, no document is
  read and no workflow is created.
- **Scripts and tools are never partially applied.** Each document either becomes a workflow with
  one published snapshot, or it does not become a workflow at all. There is no half-imported state.

## Boundaries (non-goals)

- **Bare `.json` workflow files are not imported here.** The workflow editor has its own import for
  an exported file the user picks directly. This path is the archive path only.
- **Workflows are imported, never exported.** Nothing writes workflow documents back into a
  package.
- **No marketplace section yet.** The kind exists for local `.orax` import; a registry listing for
  it is a separate concern.

See [Workflow](workflow.md) for the definition, draft/publish lifecycle, and run CRUD this import
feeds into, and [Desktop Runtime](desktop-runtime.md) for the plugin data directory layout.
