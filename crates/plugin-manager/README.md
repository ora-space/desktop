# Ora Plugin Manager

`ora-plugin-manager` discovers installed Ora plugin packages from an Ora data directory and
orchestrates checksum-verified installs of new plugin releases.

## Responsibilities

- Scan `<data-dir>/plugins/installed/<namespace>/<name>/<version>`, parse version directory names
  as SemVer, and select only the highest version for each namespace/name pair without falling back
  when that selected package is invalid.
- Read the selected package's `orax.toml` through `ora-plugin-manifest`, which owns the manifest
  schema, and require the manifest version to match the version directory.
- Resolve the fixed `main.js` entrypoint at the package root as an existing regular file whose
  canonical target remains inside its package, then retain its normalized portable relative path.
- Keep `kind` and its contribution in one value (`PluginContribution::Agent` or `::Ui`), so a
  validated plugin always carries exactly what its kind promises.
- Apply the host's surface policy to `[[ui.surfaces]]`: entry URL scheme and host allow lists
  for remote sites, the on-disk asset directory and entry document for panels, surface count and
  title limits, and id uniqueness, producing typed values (`SurfaceId`, `HostName`, `Url`,
  `PanelSource`) that downstream surface hosts reuse directly.
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

`ora-plugin-manifest` guarantees the shape of a manifest: field types, unknown fields, the id
grammar, enum spellings, that `[ui]` is present exactly for `kind = "ui"`, that every surface has
a slug `id` and a non-empty `title`, and that each `source.kind` carries its own fields. This
crate adds what depends on the host or on the package on disk:

- `kind = "workbench"` is rejected; only `agent` and `ui` run here.
- `main.js` must exist at the package root; the manifest has no entrypoint field.
- `display_name` is the plugin `name` for every kind; a ui plugin's user-visible entries are its
  surface titles. One agent-kind package contributes exactly one agent with no identifier of its
  own: the package's plugin id is that agent's identity everywhere in the host.
- A ui plugin declares at most eight surfaces, each with a package-unique `SurfaceId` (a slug of
  at most 32 bytes), a title of at most 64 characters without control characters, and
  `instances = "singleton"` (`"multiple"` is refused until the host supports it).
- A `remote_site` source is an `https` entry URL without credentials or port whose host must be
  covered by the union of `hosts` and `host_suffixes` (lowercase DNS names; suffixes match on
  label boundaries); at least one allow-list entry is required. `web_data.mode` defaults to
  `persistent`.
- A `panel` source names a `root` subdirectory of the package and an `.html` `entry` below it;
  both must exist at discovery time and resolve canonically inside the package, and the validated
  `PanelSource` carries the canonical asset directory so hosts can serve files under it as a
  containment root without ever exposing `orax.toml` or the plugin source. Panels always get an
  isolated persistent web profile, so declaring `web_data` on a panel is an error.
- Surfaces are returned sorted by id.

Validation failures report a stable `field_path` such as `ui.surfaces[0].source.entry`.
Structural failures (TOML syntax, unknown fields, wrong types, unknown `source.kind`) are
reported as `invalid_toml` with the TOML path of the offending value; semantic failures, from
either crate, as `invalid_manifest`. The `surface` module (`SurfaceId`, `HostName`,
`InstancePolicy`, `WebDataPolicy`) is the single definition of these value types; surface hosts in
other crates reuse it rather than redefining validation.

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
