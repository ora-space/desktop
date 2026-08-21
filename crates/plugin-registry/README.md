# ora-plugin-registry

`ora-plugin-registry` syncs marketplace source repositories over Git and builds the lightweight
`registry_index.json` that lists available plugins for consumers such as the UI.

## Responsibilities

- `RegistrySync::sync` clones a marketplace source when absent, otherwise fetches, checks out the
  tracked branch, and fast-forwards it against its remote through an injected `gitlancer::Git`.
- `RegistryIndex::build` recursively scans a directory for `orax.toml` files, parses each valid
  manifest into a `RegistryEntry`, and returns a deterministically ordered index built at an
  injected Unix timestamp.
- `RegistryIndex::load` reads a previously written index file; `RegistryIndex::write` replaces the
  target file atomically through `ora-utils` so readers never observe a partial index.
- Each entry's optional `logo.svg`, read from the directory holding its `orax.toml` and accepted by
  `ora-utils::svg`, is inlined into the index so consumers can render the listing from the cached
  index alone. A missing, unreadable, or unsafe icon leaves the entry listed without one.
- A single malformed or unreadable `orax.toml` is skipped, logged as a warning, and reported through
  `RegistryBuild::skipped` without blocking the whole build.

## Non-responsibilities

- Installing, enabling, disabling, or removing plugins.
- Resolving dependency trees or evaluating host version requirements.
- Choosing where source checkouts live under the data directory; callers supply the checkout path.

## Public interface

`RegistryIndex::build(dir, updated_at)` returns a `RegistryBuild` carrying the ordered index and any
skipped manifests. `RegistryIndex::load(path)` / `RegistryIndex::write(path)` read and atomically
persist an index. `RegistrySync::sync(&git, &source)` returns the checkout directory so callers can
then build an index from it.
