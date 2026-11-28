# Marketplace

English | [中文](marketplace.zh.md)

The marketplace lists plugins published in Git repositories ("sources"), installs and updates them,
and installs packs of plugins together. Source checkouts and the index live under
`~/.ora/plugins/sources/` and `~/.ora/plugins/cache/registry_index.json`. Index building belongs to
`ora-plugin-registry`; source configuration, sync and installation are composed in `ora-backend`.
The package model is in [Plugin system](plugin-system.md).

## Sources

A source is an HTTPS Git URL, a branch (default `main`), a proxy switch, an `enabled` flag and an
artifact retrieval mode. Operations: `listMarketplaceSources`, `addMarketplaceSource`,
`updateMarketplaceSource`, `deleteMarketplaceSource`.

- **Namespace.** Each source's canonical URL (case, credentials, default port, trailing slash and
  `.git` normalized) is bound once to a namespace in `plugin_source_namespace`. The binding outlives
  the source configuration: once a plugin from that source is installed, the namespace is frozen into
  its install path, data directory and Skill rows, so deleting and
  re-adding a repository reuses it. Two sources publishing the same identifier produce two distinct
  plugin ids and both stay listed.
- **Origin labels.** Every plugin and plugin Skill carries an `origin` the host computes from its
  namespace: `local` → local package, `official` → the default first-party marketplace, anything else
  → a third-party marketplace. The marketplace label is display text, recomputed per query against
  every recorded binding plus the default marketplace's canonical URL, never persisted: a unique
  repository name is shown alone, namesakes are told apart by the first segment from the end that
  differs (`segment/name`, or `segment/…/name`, host and port included when needed), and a group whose
  labels would still collide falls back to full URLs. Adding a namesake source relabels the others on
  the next query without moving any namespace or plugin id. A namespace whose binding cannot be read
  degrades to its slug with a warning instead of failing the list. The chat `/` menu, the Skills page
  and marketplace cards render this label; the plugin detail page shows the full canonical source URL
  with a copy button (the official marketplace shows "Ora official marketplace"). See
  [Origin presentation](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/registry/20261127-plugin-origin-presentation.md).
- **Enabled.** Disabling keeps the URL, branch, proxy policy, position and namespace binding but
  drops the source from sync, listing and installation.
- **Proxy.** Sources with proxy routing enabled follow the [Desktop proxy mode](../desktop/settings.md#network-proxy).
  Other sources always connect directly. Git and artifact downloads honor the same policy. Ora passes
  the selected route to Git for each operation. Explicitly direct sources keep Git's redirect behavior.
  When a destination-dependent route cannot be reapplied after a redirect, configure the final Git URL.
- **Artifact retrieval.** `Direct HTTPS` (the default) fetches absolute HTTPS release locators
  without signing. `S3 SigV4` accepts object keys, or path-style HTTPS locators on the configured
  endpoint and bucket, and signs requests with the source's region and static credential pair; it
  rejects foreign HTTPS locators instead of falling back to unsigned retrieval. Endpoint, bucket,
  region and both credentials are one complete configuration. Credentials are write-only: the source
  query never returns them and debug output redacts them. They are stored in the local SQLite
  database as plaintext configuration; there is no OS keychain, temporary credential or provider
  credential chain. Never serialize credential-update requests into logs or telemetry. Signing
  authenticates the request but does not establish integrity: every artifact must still match the
  release's SHA-256.

Errors are specific: `marketplace_s3_credentials_required` when enabling S3 without credentials,
`marketplace_artifact_retrieval_field_invalid` with a `field` parameter (never its value). Corrupt
persisted retrieval configuration is an internal error.

### The source dialog

In Settings → Plugins → Manage marketplace, **Add source** opens a dialog focused on the URL field.
Choosing S3 SigV4 reveals Endpoint, Bucket, Region and both credentials. The Backend validates
repository identity, branch syntax, S3 fields and conflicts. Enter submits; Cancel, Escape and the
close button discard the draft without closing Settings. While submitting, duplicate requests and
dismissal are blocked; failures stay in the dialog with the draft intact. If writing the S3
credentials fails, the new source is removed rather than left on Direct HTTPS. **Edit** can change
retrieval later, and leaving both credential fields blank keeps the stored pair.

## Sync and the index

`syncAvailablePlugins` pulls every enabled source and rebuilds the index; `listAvailablePlugins`
reads the cached index; `readPluginReadme` reads a listing's README.

- A source is cloned when absent, otherwise fetched, checked out and fast-forwarded. Each source's
  `registry/` directory is scanned for `orax.toml` manifests; a malformed one is skipped with a
  warning. Entries carry the publishing source's canonical URL (to tell same-named listings apart),
  the display title, an inlined `logo.svg` when it is safe, and the release's supported targets so
  the UI can disable an incompatible install before downloading anything.
- Sync is per source. A source that fails keeps exactly the listings the previous index had for it,
  without re-scanning its possibly half-written checkout, until a later sync succeeds. Only a removed
  or disabled source loses its listings, so "removed" and "unreachable" never look the same. A
  failure belongs to one source when its cause does (its Git work or a proxy it opts into); only
  failing to read Ora's own state (sources and namespace bindings) fails the whole sync.
- Failures are stored in the index and returned by both list and sync, so the shell names the stale
  sources. Each failure carries only Git's own `fatal:`/`error:` lines or the configuration problem,
  never the command line or local paths; the full error goes to the log.
- The sync time advances only when at least one source refreshed. A sync that reached none keeps the
  previous time (or reports "never synced"); a sync with no enabled source advances it over the empty
  catalog it writes.
- The index file is replaced atomically and is the only durable file in `plugins/cache/`.

### Automatic refresh

Desktop refreshes the index on an `ora-scheduler` job 15 seconds after startup and then every six
hours (`0 0 */6 * * *`) in local time, in development builds too because it never installs or
updates anything. A failed refresh is logged and not retried until the next tick.

The backend admits one rebuild at a time. A caller that arrives during a rebuild, including a user
pressing Sync, is answered from the cached index instead of queued. The shell disables Sync while an
automatic refresh runs, reported as the native event `marketplace-auto-sync-changed` (`started`,
`finished`); admission is claimed before `started` is emitted. `finished` invalidates the listing
query, which is why the shell mounts that bridge at its root.

## Installing and updating

`installPlugin` and `updatePlugin` resolve the listing against the source that owns its namespace
(so its retrieval mode and proxy apply), select the release for the host target, download through
the common pipeline with the source's proxy and retrieval mode, verify the SHA-256 while
downloading and commit the package, see [Plugin system](plugin-system.md#operations). Plugin
packages may bundle agent CLIs of hundreds of megabytes, so they have their own extraction budget.

## Packs

A `kind = "pack"` listing is an orchestration entry, not a package. Its `[[pack.members]]` name bare
identifiers that resolve inside the pack's own source; each member may name the agents it adapts,
which is display-time filtering only.

- **Install.** A static preflight finds every problem before the first member downloads a byte; a
  preflight failure has no side effects. Applicable members then install in declaration order
  through the ordinary single-plugin chain. Members already installed are skipped and recorded as
  `pre_existing`. A member failure stops the run, the members it created are rolled back, and the
  outcome reports the failing member and any member whose rollback failed. No Hook `init` runs for
  pack members, because running a program a member ships needs its own authorization.
- **Ownership journal.** Migration `0014` records, per pack, which members it created
  (`managed_by_pack`) or found (`pre_existing`) and the version each had at install time.
  `listPackInstallations` reconciles that journal with the installed tree after every restart and
  reports each member as present at the recorded version, at another version (the user took it over)
  or missing. Reconciliation only identifies drift; it repairs nothing and never consults the current
  pack listing.
- **Uninstall.** `packUninstallPlan` computes, from the journal and reconciliation, which members
  are removable (created by the pack and still at the recorded version), which are preserved and why
  (`pre_existing`, `version_changed`) and which are already missing. Execution runs the ordinary
  uninstall per member and releases journal entries one at a time, so a failure mid-run stays
  retryable and user-owned plugins are never touched.

## Decisions

- [Registry distribution](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/registry/0-registry-distribution.md)
- [Extension packs](https://github.com/ora-space/specs/blob/main/decisions/desktop/plugin/extension-pack/0-extension-pack.md)
