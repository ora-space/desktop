# ora-plugin-manifest

`ora-plugin-manifest` parses and validates the `orax.toml` manifest of one Ora plugin, in its
marketplace release form (`PluginManifest::parse`) and in the form shipped inside an installed
package (`PluginManifest::parse_installed`). It accepts caller-provided text and returns an
immutable domain object whose public types preserve the schema's semantic invariants. Both
forms accept an optional human-readable `title` that falls back to the identifier when omitted.

## Responsibilities and boundaries

- Reject malformed TOML, missing or unknown fields, unsupported resolver versions, and invalid
  field values with structured errors.
- Model plugin identifiers (`identifier`), source categories, plugin kinds (`workbench`, `agent`,
  `webview`, `skill`), HTTPS URLs, SHA-256 digests, optional source repository metadata, and
  optional Ora host version requirements as validated values.
- Pair kind-specific sections with the matching `kind`: optional `[workbench]` (page-visible
  method names) for workbench plugins, required `[webview]` (`start_url`, `allowed_origins`,
  download policy) for webview plugins. Agent and skill plugins reject both sections.
- Report structural failures with the TOML path of the offending value and semantic failures with
  a typed `ManifestField`, including the index of a webview origin or download rule.
- Preserve deterministic validation order so callers receive a stable first error.
- Reuse domain-free slug and Git branch-name validation from `ora-utils`.

## Non-responsibilities

- No filesystem access, fixed manifest filename, source-path diagnostics, or input-size policy.
- No network access, download, repository probing, or release checksum calculation.
- No plugin installation, discovery, execution, update selection, or integration with
  `ora-plugin-manager`.
- No host policy for kind-specific packages: workbench page files on disk, webview origin
  coverage, shadowed download rules, and forbidden entrypoints are checked by
  `ora-plugin-manager`, which owns the package on disk.
- No serialization, source rewriting, comment preservation, or compatibility with older formats.

## Public boundary

`PluginManifest::parse(&str)` and `PluginManifest::parse_installed(&str)` are the only manifest
construction entrypoints. Manifest fields stay private and are exposed through read-only
accessors. Reusable validated value types additionally
implement `FromStr`; none of the APIs provide an unchecked constructor.
