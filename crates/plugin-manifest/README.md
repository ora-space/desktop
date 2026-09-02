# ora-plugin-manifest

`ora-plugin-manifest` parses and validates the `orax.toml` manifest of one Ora plugin, in its
marketplace release form (`PluginManifest::parse`) and in the form shipped inside an installed
package (`PluginManifest::parse_installed`). It accepts caller-provided text and returns an
immutable domain object whose public types preserve the schema's semantic invariants. Both
forms accept an optional human-readable `title` that falls back to the identifier when omitted.

## Responsibilities and boundaries

- Reject malformed TOML, missing required fields, unsupported resolver versions, and invalid field
  values with structured errors.
- Ignore fields the schema does not know, at every nesting depth. Manifests are written by
  third-party authors and read by many Ora versions at once, so rejecting an unrecognized key would
  make each purely additive schema change delete the whole listing from an older client's
  marketplace. Additive change is absorbed here; breaking change is gated by `resolver`. The price
  is that a misspelled optional key now reads as an absent one — the entry becomes uninstallable or
  loses its icon instead of failing to parse — which is visible in publishing and is the better
  half of the trade. Required fields are not relaxed with it.
- Model plugin identifiers (`identifier`), plugin kinds (`workbench`, `agent`,
  `webview`, `skill`, `mcp`, `hook`), HTTPS URLs, SHA-256 digests, optional source repository
  metadata, and optional Ora host version requirements as validated values.
- Pair kind-specific sections with the matching `kind`: optional `[workbench]` (page-visible
  method names) for workbench plugins, required `[webview]` (`start_url`, `allowed_origins`,
  download policy) for webview plugins. Agent, skill, MCP, and hook plugins reject both sections.
- Model the resolver-one release source as a mutually exclusive union: one universal `url` +
  `sha256` pair installable on every host, or one or more unique `[[targets]]` entries each carrying
  an exact Rust target triple (`HookTarget`) from a known rustc allowlist, URL, and digest. The
  targeted form is limited to the kinds that ship a native binary of their own
  (`PluginKind::may_ship_targeted_artifact`): `hook`, which _is_ that binary, and `agent`, which
  may bundle the CLI it drives rather than requiring the user to install one. An installed
  targeted package carries an `[artifact]` section self-declaring its target so online install and
  local import apply the same host-compatibility check; the target is never part of plugin
  identity. That section is mandatory only for `hook` — an `agent` that resolves its CLI from PATH
  is a legitimate universal package with no target to declare. Universal and targeted forms may
  not coexist. Which form an agent's package was built from is not a build-time fact for the plugin
  code inside it; it learns that at spawn time from `ora-plugin-lifecycle` instead.
- Report structural failures with the TOML path of the offending value and semantic failures with
  a typed `ManifestField`, including the index of a webview origin or download rule.
- Preserve deterministic validation order so callers receive a stable first error.
- Reuse domain-free slug and Git branch-name validation from `ora-utils`.

## Non-responsibilities

- No namespace. A manifest never names the namespace its plugin is installed under: manifests are
  third-party editable content, and a namespace decides which install directory, private data
  directory, and Skill rows a package owns. The host derives it from the marketplace source that
  published the entry (`ora-domain::PluginNamespace`), so a residual `namespace` key in an older
  manifest is just another ignored unknown field.
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
