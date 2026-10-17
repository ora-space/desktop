# Agent configuration declaration compiler

English | [中文](agent-configuration-declarations.zh.md)

`ora-plugin-config::compile_agent_configuration` validates an immutable Agent declaration without
reading user configuration, running a process, or making a network request. Since the release
consumers landed, the installed configuration dispatcher recognizes this shape and package
validation compiles it for `agent`-kind packages; the local configuration store and its editor
still serve only the version-one shapes. Successful compilation is not installation,
authorization, or runtime readiness.

## Input

```json
{
  "schemaVersion": 2,
  "agent": {
    "modelAccess": {
      "contractVersion": 1,
      "protocols": ["anthropic-messages"],
      "modelCardinality": "single"
    }
  },
  "settings": {
    "verbose": {
      "type": "boolean",
      "title": "Verbose output",
      "description": "Include additional output detail",
      "default": false
    }
  }
}
```

- `schemaVersion` must be `2`; the model-access `contractVersion` must be `1`.
- The protocol list must be nonempty and contain no duplicates. Its exact names are
  `openai-completions` and `anthropic-messages`. Provider names such as `openai` and unsupported
  protocols such as `openai-responses` are rejected, not normalized to a supported protocol.
- `modelCardinality` must be `single`. The declaration cannot authorize an additional model.
- `settings` may be omitted. If present, it must be a nonempty object of the existing scalar
  Setting declarations; explicit `null` is not omission. Types remain string, number and boolean.
- Connection selections, API keys, runtime headers and upstream URLs do not belong in
  `agent.modelAccess`. Unknown members are rejected at every level. MCP `transport` and Hook
  contributions cannot be mixed into an Agent declaration.
- The existing 256 KiB source-size limit, 128-Setting limit, Setting IDs, text bounds, required
  flags, ordering and typed-default validation apply. Duplicate JSON keys are rejected before
  deserialization can collapse them.

Settings here are author-supplied metadata, not a secret store or a runtime expression language.
The compiler does not infer whether arbitrary text is a credential. Credential delivery and the
interpretation of supported non-secret options belong to separately authorized consumers.

## Compiled identity and errors

The result contains typed model capabilities and deterministically ordered Setting declarations.
Its `fingerprint` is the lowercase SHA-256 of the exact input bytes. Whitespace changes this
identity; do not reserialize a release copy before checking it against the package. Existing
version-one declaration fingerprints keep their canonicalized semantics.

Errors distinguish oversized or invalid JSON, duplicate keys, invalid structure/Settings,
unsupported schema/contract versions, and empty, repeated or unsupported protocols. Error display
does not include rejected strings or source snippets. Compilation does not prove source namespace,
release version/target, archive digest, package containment, personal ownership or live model access.

## Verification

The [shared vectors](../crates/plugin-config/tests/fixtures/agent-configuration-contract.json) carry
literal `source` strings and expected compiled objects or error categories. Source strings keep
the same bytes on LF and CRLF checkouts. The Rust tests use the public compiler and additionally
check raw-byte and Setting-count boundaries, nested duplicate keys, invalid UTF-8, safe diagnostics,
legacy fingerprint behavior and the dispatcher's shape routing. The Cloud marketplace sync ships
a Go mirror of this compiler constrained by the same vector file; keeping the fixture and its
Cloud copy in one change is what makes the two languages drift-proof.

Run `cargo test -p ora-plugin-config --locked` and
`cargo clippy -p ora-plugin-config --locked -- -D warnings` for this crate.

## Release association and install verification

A marketplace release manifest carries the publication association in a release-form-only
`[agent_configuration]` section: the declaration schema version, the SHA-256 of the exact
declaration bytes every package of the release must contain, and the registry-entry-relative
path of the static copy the marketplace sync reads. Only an `agent` listing that declares a
release source may carry it, the installed manifest form never does, and unknown keys inside the
section fail the listing. The copy-path grammar excludes separators, escapes, absolute locations,
parent segments and non-ASCII, so a validated path resolves containably inside the checkout.

Both install paths prove the association against the package's own bytes before anything
commits. The marketplace path derives the expectation from the listing's section; the Node path
receives it frozen on Cloud's install plan. The package's in-archive `assets/config.json` must
compile and fingerprint to exactly the associated digest, or the package is rejected while it is
still staged. A package may carry a declaration without an association — it is inert package
metadata until a release binds it — and a mismatched association never installs as that release.
