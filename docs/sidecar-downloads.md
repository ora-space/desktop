# Verified desktop sidecars

The Desktop frontend build runs `scripts/setup-binary.ts` before bundling. It
checks Deno and ripgrep in `apps/desktop/src-tauri/binaries` against the selected
release's executable SHA-256, and runs `--version` for a native target. A stale,
corrupt, or unexecutable file is replaced only after a new download passes all
checks. Foreign target executables are authenticated by their pinned digest,
without trying to run them on the build host. The ora-reaper sidecar is built
from the checked-out Rust source by `task build:desktop`.

`scripts/sidecar-checksums.json` is the reviewed trust source for downloads. Its
keys bind exact official GitHub release URLs to archive and executable SHA-256
values. The initial pins cover Deno 2.9.5 and ripgrep 15.2.0 on Linux x64, macOS
arm64/x64, and Windows x64. Archive hashes were checked against the official
GitHub release asset `digest` fields; executable hashes were calculated from
those verified archives:

- [Deno release](https://github.com/denoland/deno/releases/tag/v2.9.5)
- [ripgrep release](https://github.com/BurntSushi/ripgrep/releases/tag/15.2.0)

Each installation creates its own temporary directory inside `binaries`, keeping
staging on the destination filesystem. The archive must match its pin before
extraction. The extracted regular executable must match its pin and, on native
targets, report the expected version before an atomic rename publishes it.
Failures preserve the previous binary and fail the build. Cleanup removes only
that installation's staging directory. Concurrent installers use independent
archives and extraction trees and publish only verified bytes. Existing files
and legacy `.extract-*` directories are not used as staging or deleted.

## Updating a sidecar

1. Select the official release. Update `.deno-version` and `engines.deno` for
   Deno, or the ripgrep version in setup and CI for ripgrep.
2. Read the release asset digests from the official repository's GitHub release
   API (`/repos/denoland/deno/releases/tags/v<VERSION>` or
   `/repos/BurntSushi/ripgrep/releases/tags/<VERSION>`), and compare them with the
   published checksum assets where available.
3. Download each supported archive and verify its SHA-256 before extracting it.
   Hash the extracted `deno`/`rg` executable (with `.exe` on Windows) and update
   both hashes under the exact release URL in `scripts/sidecar-checksums.json`.
4. Review the pin changes and run `deno task test:tooling`. Exercise the real
   download with `deno task setup:binary --force`, and run the desktop build CI
   on its supported platforms.

Unknown releases or targets, missing pins, and malformed digests fail closed.
There is no checksum override or unverified fallback. `--force` bypasses cache
reuse only; it does not bypass integrity or version checks.
