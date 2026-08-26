# Ora Plugin Manager

`ora-plugin-manager` discovers installed Ora plugin packages from an Ora data directory and
orchestrates checksum-verified installs of new plugin releases.

## Responsibilities

- Scan `<data-dir>/plugins/installed/<namespace>/<name>/<version>`, parse version directory names
  as SemVer, and select only the highest version for each namespace/name pair without falling back
  when that selected package is invalid.
- Read the selected package's `orax.toml` through `ora-plugin-manifest`, which owns the manifest
  schema, and require the manifest version to match the version directory.
- Resolve the fixed `main.js` entrypoint for agent and workbench packages as an existing regular
  file whose canonical target remains inside its package, then retain its portable relative path.
  Webview, skill, and MCP packages have no process entrypoint.
- Keep `kind` and its contribution in one value (`PluginContribution::Agent`, `::Workbench`,
  `::Webview`, `::Skill`, or `::Mcp`), so a validated plugin always carries exactly what its kind
  promises. Skill contributions carry no additional contract fields, but the package must contain
  one or more `assets/<name>/SKILL.md` trees. Each Skill manifest is parsed and its declared name
  must match the package directory before it can be cataloged. MCP contributions carry the
  immutable Installed MCP Descriptor compiled from `assets/config.json`.
- Compile an optional `assets/config.json` through `ora-plugin-config` without treating the
  package directory as a data root, and record whether the immutable declaration is absent, valid,
  or invalid so lifecycle can refuse to enable a broken package. A transport-bearing (MCP-shaped)
  `assets/config.json` is only accepted on `kind = "mcp"` packages, and an MCP package requires
  one.
- Apply host-side workbench and webview policy that the manifest crate cannot check: a workbench
  package must ship `assets/index.html` beside `main.js`; a webview package must not ship
  `main.js`, must cover `start_url` with `allowed_origins`, and must not declare shadowed
  download rules.
- Read the package's optional `logo.svg` icon and retain its source text once
  `ora-utils::svg` accepts it. A package without an icon is ordinary; an icon that is present but
  unreadable or unsafe becomes a discovery issue and leaves the plugin itself discovered without one.
- Return a deterministic, immutable snapshot of valid installed plugins keyed by
  `ora_domain::PluginId`.
- Isolate malformed or unsupported packages as structured discovery issues.
- Install a plugin release: download the `.orax` package (through an injected `ora-utils::http`
  `HttpDownload`), verify its SHA-256 while downloading, and safely extract it into
  `<data-dir>/plugins/installed/<namespace>/<name>/<version>` with `ora-utils::archive`.
- Import one local `.orax` release archive by extracting into a disposable staging directory,
  parsing its in-archive `orax.toml`, verifying a declared `sha256`, and then moving only the
  validated tree into `<data-dir>/plugins/installed/<namespace>/<name>/<version>`.

## Non-responsibilities

- Parsing the manifest schema; `ora-plugin-manifest` does that and this crate consumes the result.
- Enabling, disabling, or removing plugins; starting plugin processes or loading plugin JavaScript.
- Evaluating the `[dependencies].ora` requirement.
- Watching the filesystem after discovery completes.

## Public interface

Call `PluginManager::discover(data_dir)` once during application bootstrap. Consumers read the
resulting snapshot through `installed_plugins()` and report any non-fatal problems from
`discovery_issues()`. `installed_root(data_dir)`, `MANIFEST_FILE_NAME`, and
`INSTALLED_ENTRYPOINT` expose the layout to callers that write or inspect it. A local `.orax`
archive is imported with `Installer::install_local(archive_path, data_dir)`, which returns an
`InstalledPackage` carrying the materialized `package_dir` and the `namespace/name` plugin id
derived from the in-archive manifest. `Installer::new` accepts any `HttpDownload`; `install`
returns the package directory it extracted into.

## Validation split

`ora-plugin-manifest` guarantees the shape of a manifest: field types, unknown fields, the
`identifier` grammar, enum spellings, that `[webview]` is present exactly for `kind = "webview"`,
and that `[workbench]` is refused for every other kind. This crate adds what depends on the host
or on the package on disk:

- Agent and workbench packages must contain `main.js`; webview and MCP packages must not. Skill
  packages have no process entrypoint.
- A workbench package must ship `assets/index.html`; the canonical `assets/` directory is the
  only tree ever served to the page.
- A webview package is configuration-only: `start_url` must belong to `allowed_origins`, origins
  cannot repeat, and a download rule whose page set is covered by an earlier rule is rejected.
- A skill package must contain `assets/` with at least one immediate Skill directory. Every
  immediate directory is one Skill package rooted by `SKILL.md`; the plugin has no intermediate
  `skills/` directory. The Skill catalog namespace is the owning plugin's canonical
  `<namespace>/<identifier>` identity. Every such directory must contain a regular root
  `SKILL.md`. Optional `scripts/`, `references/`,
  and nested `assets/` contents are preserved but not interpreted in this release.
- An MCP package is configuration-only: it must not ship `main.js`, must provide an MCP-shaped
  `assets/config.json` (settings subset plus exactly one transport), and, for a stdio transport,
  its command must resolve to a regular file inside the package (executable on Unix).
- `display_name` is the plugin identifier for every kind. One agent-kind package contributes
  exactly one agent with no identifier of its own: the package's plugin id is that agent's
  identity everywhere in the host.

Validation failures report a stable `field_path` such as `webview.allowed_origins[0]`.
Structural failures (TOML syntax, unknown fields, wrong types) are reported as `invalid_toml`
with the TOML path of the offending value; semantic failures, from either crate, as
`invalid_manifest`.

## Layout rules

Namespace, package, and version directories must be real directories (symlinks are skipped at
every level, because the installer only ever writes real directories) and the manifest inside a
version directory must be a regular file. A version directory that is not valid SemVer is reported
as `invalid_install_path`. The directory names are not part of a package's identity: the manifest
alone names the plugin, and two packages claiming the same `<namespace>/<name>` keep the first in
path order and report the second as `duplicate_plugin_id`. Discovery never recurses below one
version directory and never reads more than 1 MiB from one manifest. The pre-versioned
`<data-dir>/plugins/<package>` layout is not discovered or migrated. Entrypoint containment
rejects the current target of a package-escaping symlink, but path-based validation cannot prevent
a concurrent symlink replacement between discovery and later loading. A missing installed root
represents an empty installation and is not an error.
