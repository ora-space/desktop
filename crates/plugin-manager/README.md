# Ora Plugin Manager

`ora-plugin-manager` discovers installed Ora plugin packages from an Ora data directory using the
orax package shape, and orchestrates installing new plugin releases.

## Responsibilities

- Scan `<data-dir>/plugins/installed/<namespace>/<name>/<version>`.
- Parse version directory names as SemVer and select only the highest version for each
  namespace/name pair without falling back when that selected package is invalid.
- Read the selected package's `orax.toml` and parse it through `ora-plugin-manifest`.
- Resolve the fixed `main.js` entrypoint as an existing regular file whose canonical target remains
  inside its package, then retain its normalized portable relative path.
- Normalize plugin identity to `namespace/name` and retain the validated orax metadata needed by the
  lifecycle layer.
- Read the package's optional `logo.svg` icon and retain its source text once
  `ora-utils::svg` accepts it. A package without an icon is ordinary; an icon that is present but
  unreadable or unsafe becomes a discovery issue and leaves the plugin itself discovered without one.
- Return a deterministic, immutable snapshot of valid installed plugins.
- Isolate malformed or unsupported packages as structured discovery issues.
- Install a plugin release: download the `.orax` package (through an injected `ora-utils::http`
  `HttpDownload`), verify its SHA-256 while downloading, and safely extract it into
  `<data-dir>/plugins/installed/<namespace>/<name>/<version>` with `ora-utils::archive`.
- Import one local `.orax` release archive by extracting into a disposable staging directory,
  parsing its in-archive `orax.toml`, verifying a declared `sha256`, and then moving only the
  validated tree into `<data-dir>/plugins/installed/<namespace>/<name>/<version>`.

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

Import a local `.orax` release archive with `InstallError`-returning
`Installer::install_local(archive_path, data_dir)`, which returns an `InstalledPackage` carrying
the materialized `package_dir` and the `namespace/name` plugin id derived from the in-archive
manifest.

Discovery never follows symlinked package directories and never reads more than 1 MiB from one
manifest. The manifest version must match the selected version directory. Entrypoint containment
rejects the current target of a package-escaping symlink, but path-based validation cannot prevent a
concurrent symlink replacement between discovery and later loading. A missing installed-plugins
directory represents an empty installation and is not an error. The legacy
`<data-dir>/plugins/<package>` layout is not discovered or migrated.
