# Ora Plugin Manager

`ora-plugin-manager` discovers installed Ora plugin packages from an Ora data directory.

## Responsibilities

- Scan direct child directories under `<data-dir>/plugins`.
- Read and validate each child package's `package.json`.
- Resolve `ora.main` as an existing regular file whose canonical target remains inside its package,
  then retain its normalized portable relative path.
- Pair `ora.kind` with the contribution that kind must declare, so a validated plugin always
  carries exactly the contribution its kind promises.
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

Discovery never follows symlinked package directories, never recurses below one package directory,
and never reads more than 1 MiB from one manifest. Entrypoint containment rejects the current target
of a package-escaping symlink, but path-based validation cannot prevent a concurrent symlink
replacement between discovery and later loading. A missing plugins directory represents an empty
installation and is not an error.
