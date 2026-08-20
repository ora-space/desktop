# Ora Plugin Manager

`ora-plugin-manager` discovers installed Ora plugin packages from an Ora data directory using the
orax package shape, and orchestrates installing new plugin releases.

## Responsibilities

- Scan direct child directories under `<data-dir>/plugins/installed`.
- Read each child package's `orax.toml` and parse it through `ora-plugin-manifest`.
- Resolve the fixed `main.js` entrypoint as an existing regular file whose canonical target remains
  inside its package, then retain its normalized portable relative path.
- Normalize plugin identity to `namespace/name` and retain the validated orax metadata needed by the
  lifecycle layer.
- Return a deterministic, immutable snapshot of valid installed plugins.
- Isolate malformed or unsupported packages as structured discovery issues.
- Install a plugin release: download the `.orax` package (through an injected `ora-utils::http`
  `HttpDownload`), verify its SHA-256 while downloading, and safely extract it into
  `<data-dir>/plugins/installed/<name>` with `ora-utils::archive`.

## Non-responsibilities

- Enabling, disabling, or removing plugins (those reach through the lifecycle layer).
- Starting plugin processes or loading plugin JavaScript.
- Choosing a concrete network transport: the installer consumes the `HttpDownload` trait, so
  production wiring supplies a network downloader and tests/offline installs use the local one.
- Resolving plugin dependency graphs or evaluating host-version requirements at discovery time.
- Watching the filesystem after discovery completes.

## Public interface

Call `PluginManager::discover(data_dir)` once during application bootstrap. Consumers read the
resulting snapshot through `installed_plugins()` and report any non-fatal problems from
`discovery_issues()`.

Build an `Installer::new(downloader)` with any `HttpDownload` implementation and call
`install(&manifest, source, data_dir)`, passing a `DownloadSource::Url(...)` for online installs or
a `Local` path for offline and test installs.

Discovery never follows symlinked package directories, never recurses below one package directory,
and never reads more than 1 MiB from one manifest. Entrypoint containment rejects the current target
of a package-escaping symlink, but path-based validation cannot prevent a concurrent symlink
replacement between discovery and later loading. A missing installed-plugins directory represents an
empty installation and is not an error.
