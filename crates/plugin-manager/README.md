# Ora Plugin Manager

`ora-plugin-manager` discovers installed Ora plugin packages from an Ora data directory.

## Responsibilities

- Scan direct child directories under `<data-dir>/plugins`.
- Read and validate each child package's `package.json`.
- Resolve `ora.main` as an existing regular file whose canonical target remains inside its package,
  then retain its normalized portable relative path.
- Pair `ora.kind` with the contribution that kind must declare, so a validated plugin always
  carries exactly the contribution its kind promises (`agent` or `ui`).
- Validate ui surface declarations, including entry URL scheme and navigation allow lists, into
  typed values (`SurfaceId`, `HostName`, `Url`) that downstream surface hosts reuse directly.
- Return a deterministic, immutable snapshot of valid installed plugins.
- Isolate malformed or unsupported packages as structured discovery issues.

## Non-responsibilities

- Installing, enabling, disabling, or removing plugins.
- Starting plugin processes or loading plugin JavaScript.
- Evaluating Ora, Bun, or plugin API engine ranges.
- Watching the filesystem after discovery completes.

## Public interface

Call `PluginManager::discover(data_dir)` once during application bootstrap. Consumers read the resulting snapshot through `installed_plugins()` and report any non-fatal problems from `discovery_issues()`.

One agent-kind package contributes exactly one agent, declared under `ora.contributes.agent`. The
agent has no identifier of its own: the package's `ora.id` is that agent's identity everywhere in
the host, which is why `PluginContribution::Agent` carries only a display name and contract version.
A package whose `ora.kind` is `agent` but which declares no agent fails validation.

One ui-kind package contributes one to eight surfaces under `ora.contributes.ui.surfaces`, exposed
as `PluginContribution::Ui`. Each surface has a package-unique `SurfaceId` (a slug of at most 32
bytes), a trimmed title, a singleton instance policy, and a `remoteSite` source: an `https` entry
URL without credentials or port whose host must be covered by the union of `navigation.allowHosts`
and `navigation.allowHostSuffixes` (lowercase DNS names). Surfaces are returned sorted by id. A ui
package that also declares an agent, or an agent package that declares a ui block, fails validation.
Every plugin id, regardless of kind, is one or two dot-separated slug segments of at most 64 bytes
in total, because ids become webview labels and directory names.

Validation failures report a stable `field_path` such as
`ora.contributes.ui.surfaces[0].source.entryUrl`. The `surface` module (`SurfaceId`, `HostName`,
`InstancePolicy`, `WebDataPolicy`) is the single definition of these value types; surface hosts in
other crates reuse it rather than redefining validation.

Discovery never follows symlinked package directories, never recurses below one package directory,
and never reads more than 1 MiB from one manifest. Entrypoint containment rejects the current target
of a package-escaping symlink, but path-based validation cannot prevent a concurrent symlink
replacement between discovery and later loading. A missing plugins directory represents an empty
installation and is not an error.
