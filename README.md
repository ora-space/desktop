# Ora Desktop

Ora is an IDE for AI agents. The shared Rust crates live in `crates/`,
executable Rust packages live in `apps/`, and the shared TypeScript packages
live in `packages/`. All Rust packages share the root Cargo workspace.

## Supported platforms

Ora Desktop runs on Windows and Linux. macOS is no longer supported: v0.6.0 was the last macOS
release, and macOS installations stay on it without further updates.

## Documentation

Start with the [documentation map](docs/README.md) ([中文](docs/README.zh.md)) and the
[architecture overview](docs/architecture.md). Crate and package internals live in each
directory's `README.md`; decisions live in [ora-space/specs](https://github.com/ora-space/specs).

## Development

Install the Deno version pinned in [`.deno-version`](.deno-version), Rust (see `rust-toolchain.toml`), Task, Git, ripgrep,
and the Tauri system dependencies for your platform. Node.js and pnpm are not
required. Run `task install:frontend` to install the locked npm dependencies
with Deno and configure Git hooks, then `task run:desktop`.

See [AGENTS.md](AGENTS.md) for code conventions, and [Local development](docs/development/local-development.md),
[Testing](docs/development/testing.md) and [Changing the code](docs/development/changing-the-code.md)
for the details. Common commands:

- `task test` — full lint and test suite for frontend and Rust workspace
  packages (long-running)
- `task lint` — all lint tasks
- `task export-contracts` — regenerate frontend contracts and plugin protocol bindings from Rust
- `task check:contracts` — verify generated contracts without rewriting the checkout; LF and CRLF
  line endings are treated as equivalent while other content changes still fail
