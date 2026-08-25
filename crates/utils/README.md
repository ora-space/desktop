# ora-utils

`ora-utils` is the lowest-level Rust crate in the workspace: generic, domain-free building blocks
that any other crate can consume without introducing dependency cycles.

## Responsibilities and boundaries

- `path`: platform-independent relative-path parsing (`PortableRelativePath`,
  `StrictRelativePath`), canonical root containment (`CanonicalPathRoot`), and lexical
  normalization helpers.
- `archive` (Cargo feature `archive`): safe materialization of untrusted `.zip` / `.tar.gz`
  archives and folder trees into a destination directory with zip-slip defenses, encrypted and
  special-entry rejection, portable case-conflict detection, and cumulative entry/byte budgets.
- `atomic`: atomically replacing a file by writing a same-directory temporary file, optionally
  preparing its metadata, and renaming it over the destination, so readers never observe partial
  content or a destination change after preparation fails.
- `directory`: rejects links and special entries while copying or fingerprinting directory trees;
  fingerprints cover portable paths, file bytes, entry kinds, and executable permissions while
  allowing callers to exclude their own metadata files.
- `hash` (Cargo feature `validation`): streaming SHA-256 digests over a reader or file without
  buffering the whole input.
- `http` (Cargo feature `http`): the transport-agnostic `HttpDownload` contract plus an offline
  `LocalFileDownloader` that copies a local file or `file://` URL to a destination, enforcing an
  optional byte limit and SHA-256 checksum with an atomic replace.
- `http-reqwest` (Cargo feature `http-reqwest`, implies `http`): the `reqwest`-backed
  `ReqwestDownloader` that streams remote HTTP(S) responses with timeouts, retries, progress,
  cancellation, and proxy resolution driven by explicit config and `*_PROXY`/`NO_PROXY` variables.
- `html` (Cargo feature `validation`): conservative validation rejecting README text that embeds
  scriptable HTML (forbidden tags, `on*` event handlers, `javascript:`/`data:` URIs).
- `svg` (Cargo feature `validation`): security validation for SVG icons — accepts well-formed
  XML only, forbidding `<script>`/`<foreignObject>`, event-handler attributes, external `href`
  references, and files over the 50 KiB cap. `read_validated` reads one SVG file through that
  policy with a bounded read and returns its source text, so callers never hold untrusted markup.
- `fs`: portable file naming for untrusted names (`sanitize_file_name`) and collision-free name
  selection inside a directory (`next_available_file_name`).
- `Slug`: an owned lowercase ASCII slug segment with stable syntax and byte-length guarantees.
- `GitBranchName`: an owned short Git branch name validated without starting a Git process.

The `validation` feature is enabled by default because its dependencies are small and already in
the workspace dependency graph; path-only consumers can opt out with `default-features = false`.
Heavier capabilities such as `archive`, `http`, and `http-reqwest` stay opt-in.

## Non-responsibilities

- No `ora-*` dependencies and no domain vocabulary (skills, plugins, tasks, workspaces). Callers
  wrap these primitives with their own semantics and error codes.
- No async runtime in the default/light feature set; `http-reqwest` is the only feature that brings
  a runtime (`tokio`) and a transport (`reqwest`), and it requires the caller to run an async
  runtime.
- No workspace-level filesystem services; those live in `ora-fs`.

## Admission rule

Logic belongs here when it is independent of every Ora domain concept, transport, and runtime,
already has one consumer, and a second consumer could use it unchanged. Heavier optional
dependencies must be gated behind Cargo features so path-only consumers stay light.
