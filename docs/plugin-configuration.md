# Plugin Configuration

This document is the authoritative behavioural specification for the first Plugin Configuration delivery. Local files under `specs/` are exploratory design material and do not define committed behaviour.

Ora renders a host-owned configuration page from declarations shipped with an Agent Plugin. The first delivery owns declaration validation, editing, persistence, and Configuration Completeness; it does not deliver stored values to Agent Plugins or gate Agent runtime use. Secret support is also deliberately deferred.

## Declaration and values

Each plugin version may ship an immutable `assets/config.json` with `schemaVersion: 1`. Absence means Not Declared; a present declaration must contain at least one Setting. Ora validates the complete declaration during installation before committing the installation. A package discovered later with an invalid declaration has the Invalid Declaration installation state and cannot be enabled, but remains visible for diagnosis and removal.

Declaration JSON is limited to 256 KiB and 128 Settings. Parsing rejects duplicate object keys rather than accepting parser-dependent first- or last-value behaviour.

User values are plugin-global and shared across Workspaces. Ora stores them independently of installed versions in `data/<namespace>/<name>/store.json`, using atomic replacement and current-user-only file permissions. A new installation has no value file until the first successful save.

The value file contains `schemaVersion: 1`, a monotonically increasing unsigned integer `revision`, and the `values` object. An absent file has revision zero. Saves acquire a per-plugin write lock, compare the expected revision, and atomically replace the file with the next revision.

Value JSON is limited to 1 MiB, and an individual string value is limited to 64 KiB. Value parsing also rejects duplicate object keys.

Schema version 1 supports `string`, `number`, and `boolean` Setting types. Secret and other future types require a new schema version rather than silently extending version 1. Configuration pages render every declared Setting; `required` controls validation and Configuration Completeness rather than visibility. A required string is missing when trimming it produces an empty string, a number must be finite, and both boolean values are valid. Non-empty string values are stored without trimming.

A Setting ID must match `^[a-z][A-Za-z0-9]{0,63}$`. A Setting may declare an integer `order`. Explicitly ordered Settings appear first in ascending order; omitted orders appear afterward. Equal or omitted orders are resolved by Setting ID, and duplicate explicit orders are valid.

Every Setting has a plain-text title and description. Both must be non-empty after trimming; titles are limited to 120 characters and descriptions to 2,000 characters. Version 1 does not interpret Markdown or HTML.

A Setting may declare a type-correct default. Stored values take precedence over defaults, and a required Setting with a default is complete. Reset removes the stored override instead of copying the default into `store.json`.

## Configuration summary

Plugin list snapshots expose Installation Validity, Durable Eligibility, Configuration Summary, and Runtime Status as orthogonal facts. Configuration Summary is an exclusive model:

```text
NotDeclared
Available { completeness: Complete | Incomplete }
Unavailable { errorCode }
```

Only Available configuration has Configuration Completeness. Missing required values produce Incomplete without preventing installation, enablement, plugin registration, or Agent use in the first delivery. Unavailable represents configuration data that cannot be read and must not be collapsed into Incomplete.

## Editing

Plugin Manager exposes Configure for plugins that declare Settings and shows Needs Configuration for Incomplete configuration. Installation does not force the configuration page open. The first delivery does not navigate to configuration from Agent runtime surfaces.

Configure opens a third-level detail page inside the existing Settings Dialog, with Back navigation to Plugin Manager. Plugin Configuration does not introduce an application route and does not expand the form inline inside a list row.

Configuration detail fields expose the Setting Declaration, stored override, effective value, and whether the effective value is stored, defaulted, or absent. Save submits only explicit overrides; Reset removes an override. Inputs display effective defaults with a Default indication without turning untouched defaults into stored overrides. Number inputs keep a string draft and parse it during Save.

Details include a fingerprint of the current declaration as well as the value revision. Save compares both. A changed declaration produces `PluginConfigurationDeclarationChanged`, preserving the editor's draft and requiring a reload even when no value revision changed.

Boolean fields use an explicit three-state editor so absent, false, and true remain distinct. Optional or defaulted fields offer Use default or Not set, On, and Off. Required fields without a default offer an unselected placeholder, On, and Off.

The editor uses an explicit Save action. A save submits the complete set of values recognized by the current declaration together with the revision the editor loaded. Ora accepts a type-correct but incomplete replacement and reports Incomplete after saving. It rejects a stale revision with `ConfigurationRevisionConflict`, preserves the editor's unsaved values, and asks the user to reload rather than overwriting or automatically merging. A successful save validates the whole replacement and atomically persists one new revision; field changes are not auto-saved.

Leaving a dirty editor through its breadcrumb, a Settings category switch, or Settings Dialog close requires an explicit Save, Discard, or Cancel decision. Reset All is a confirmed domain operation that removes every stored override and writes a new empty-values revision; it does not delete the value file or reset the revision to zero. Defaults remain effective.

After Save, the editor remains open, adopts the returned revision and value sources, and displays Saved. The same response carries the authoritative Configuration Summary used to update the plugin-list cache without refetching every plugin. Save and Reset update query caches only; the first delivery does not publish a global configuration-changed event or poll for changes.

A malformed or unreadable value file produces `ConfigurationLoadFailed`, makes Configuration Summary Unavailable, and is not treated as Needs Configuration. Ora preserves the original file and requires an explicit reset or replacement before writing over it. After user confirmation, recovery moves the original to a unique `.corrupt-<local timestamp>[-<counter>]` backup without overwriting an earlier backup before writing a replacement.

The first delivery validates declaration constraints, value types, and Configuration Completeness only. It does not execute Agent Plugin code during Save and does not define a generic connection-test or business-validation hook.

## Module boundary

The `ora-plugin-config` crate owns declaration compilation, value-file parsing, Configuration Completeness evaluation, revision comparison, and atomic persistence. Lifecycle and backend components orchestrate its public operations without parsing configuration files themselves. Filesystem access is supplied through a testable port rather than embedded across callers.

Plugin list responses carry Configuration Summary. `getPluginConfiguration` returns editor details, `savePluginConfiguration` performs revision-checked replacement, and `resetPluginConfiguration` performs confirmed recovery or Reset All. No API exposes raw configuration files, their paths, or generic JSON file access to the frontend.

Validation failures use stable error codes and field errors keyed by Setting ID. Frontends localize the codes and focus the first invalid field rather than parsing backend message text.

## Upgrade and removal

An upgraded declaration is authoritative. Values for removed Setting IDs are not exposed, and incompatible stored types make Configuration Completeness incomplete without implicit conversion. Ora retains obsolete values to support rollback until the user next successfully saves the current declaration, at which point it removes values that the declaration no longer recognizes.

Uninstall offers an explicit option to delete configuration data and credentials, selected by default. Users may retain data for a later reinstall. When deletion is selected, Ora stops the plugin and stages both installation and data directories through same-volume atomic moves. A failed move rolls earlier moves back; after all moves succeed, uninstall is committed and staging cleanup may be retried independently.

## Deferred Secret support

Secret fields, credential-store persistence, secure input UI, and runtime Secret injection are not part of the first delivery. Future support must follow [ADR 0001](adr/0001-host-owns-plugin-secret-resolution.md): only Ora resolves Secret plaintext, and Agent Plugins never receive it.

## Deferred Agent consumption

The first delivery does not define how an Agent Plugin consumes `store.json`, resolved values, or Configuration Revisions. It does not modify Agent start contracts, block Agent use, create Configuration Snapshots, restart Agent instances, or navigate from runtime failures. [ADR 0002](adr/0002-gate-capabilities-on-configuration-readiness.md), [ADR 0004](adr/0004-bind-agent-instances-to-configuration-snapshots.md), and [ADR 0006](adr/0006-pass-configuration-snapshots-in-agent-start.md) remain proposals for that future phase.
