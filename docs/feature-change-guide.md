# Adding or removing a feature

Start with the owning use case, not the number of files. DTOs, generated outputs, tests and docs
are separate categories; multiple changed files do not by themselves indicate duplicated facts.
Use [architecture checks](architecture-checks.md) and the [refactor evidence](refactor-progress.md)
to distinguish necessary composition from repeated registration.

## Add an operation in an existing domain

1. Define its request/response DTOs in the owning `crates/contracts/src/<domain>.rs` and register
   their TypeScript exports in that same module's `export` function. Keep transport routing and
   Webview authorization out of public DTOs/manifests. A genuinely new DTO family also needs its
   one explicit Rust module/export composition entry.
2. Add the logical operation under `xtask/src/frontend/namespaces/<namespace>.rs`: its operation
   name, client namespace/member, DTO names and explicit `FrontendResponseMode::Unary` or
   `Stream`. There is no central stream-name fallback to update.
3. Add its Desktop-owned binding under `apps/desktop/src-tauri/bindings/`. Unary bindings declare
   their Rust handler and `Permission` explicitly. Stream bindings declare a domain startup
   handler and reuse the existing authorized `stream_contract`/`cancel_contract_stream` pair.
   New native commands belong to the Desktop catalog even when there is no SDK operation.
4. Implement the use case in its domain module. Backend operations use a narrow domain handle;
   do not add a root `Backend` forwarding method or expose storage, locks or supervisors. The
   Desktop adapter invokes that handle through the existing request lifecycle helpers. A file
   stream startup belongs to `commands/files.rs`; the shared `StreamStart` owns registration,
   cancellation, startup settlement and forwarding. Do not reproduce those rules in the domain.
5. Run `task export-contracts`, then `task check:contracts`. Do not edit client forwarding, DTO
   barrels, transport maps, stream enums/dispatch, command registration or permission outputs by
   hand. These are generated from the owning declarations. Repeat generation when verifying
   determinism; checks do not repair drift or delete unknown handwritten files.
6. Add tests at the operation's real interface. Backend tests retain real SQLite/Git where those
   are the behavior under test; frontend tests use the production client with typed operation
   handlers. Verify DTO and call-option propagation, error/requestId behavior and relevant
   lifecycle outcomes. Streams need consumption, abort/unmount and cleanup evidence, not just a
   test that construction returned an iterable.
7. Run the smallest relevant tests while iterating, then the required package checks. Desktop
   changes require `task test:tauri`, not only workspace tests; cross-repository refactors finish
   with `task test`. Catalog validity cannot replace compilation of the actual handler.

The four handwritten facts are **contract**, **logical operation**, **Desktop binding**, and
**business implementation**. A business implementation may legitimately span its domain handle
and host adapter. The goal is not exactly four changed files; generated files, tests and docs are
counted separately. Adding an entirely new domain may also change explicit composition roots.

## Frontend ownership checklist

- Keep translation data in the owning feature. Add a resource to the root i18n composition only
  for a new owner; retain one synchronous `appI18n` initialization and locale storage semantics.
  Resource key parity/duplicates must pass. A rendering test using translations initializes the
  instance itself and passes the clean-stderr gate without timing-based warning suppression.
- Put query identity, authoritative response adoption and invalidation in the data owner under
  `state/data/`. Preserve meaningful tuple/prefix distinctions, delete cascades and event refresh
  scopes. The UI owns selection and presentation; it must not copy query-key strings.
- Declare only needed public symbols in the feature's `interface.json`, with their purpose.
  Other symbols remain private, even in the same source file. Shared state must not depend on
  feature UI. Before exposing a private implementation, decide whether the responsibility is
  genuinely shared or the caller should use the existing public interface.
- Compose test memory adapters explicitly. Missing operations must fail; an empty success is only
  valid when the test explicitly opts into that synthetic behavior. Configure custom behavior
  through typed handlers, not replacement generated-client methods.
- Establish who owns watchers, event bridges, timers, QueryClient observers and native surfaces.
  Scope changes and unmount/shutdown must dispose the old resource. Verify cache isolation and
  actual subscription finalization. Files continues to own explicit Explorer/Search composition;
  do not add a generic view/plugin container for a feature that does not need one.

## Remove a feature

1. Identify its public consumers and durable data first. Product deletion does not authorize
   deleting user files. The filesystem layout is a hard compatibility constraint: if an old path
   can collide with the new layout or prevent safe startup, design detection/migration/isolation
   explicitly before changing it. This refactor itself changes no user-data layout.
2. Remove its owning implementation, DTOs and local DTO export entries, logical catalog entries,
   Desktop bindings and genuinely obsolete composition entries. Remove its local public-interface
   declaration and translations with the feature. Do not retain unused root forwarding or a
   second implementation as a permanent compatibility layer.
3. Remove or update consumers through their existing domain interface. For shared data, verify
   whether other owners still need its query factory and invalidation policy. Stop subscriptions
   and remove feature-owned cached/draft state without erasing another workspace's state.
4. Regenerate. Removed operations must disappear from generated clients, routing, DTO exports,
   registration and applicable permissions. Check both tracked diffs and untracked generated
   files; retain unknown handwritten files. Confirm main and plugin Webview capabilities
   separately, and keep shared stream commands while any stream still requires them.
5. Move tests with surviving responsibilities. Delete a test obligation only when the behavior is
   intentionally removed or its replacement directly covers it. Keep regression evidence for
   failure, persistence, concurrency, cancellation/recovery and shutdown where relevant.
6. Search for old operation names, imports, query keys, translation keys and fixture wiring. Run
   generation, feature-interface and Rust-size checks. Remove stale size exceptions for deleted
   modules. Update ownership docs, user-facing docs where applicable, and the evidence index.
   `specs/` is an independent repository; changes there require its own instructions and status
   inspection, not an incidental parent-repository edit.

Review the final diff by category: owning implementation/declarations, necessary composition,
generated outputs, tests, and docs. A temporary add/remove rehearsal belongs in an isolated
worktree; remove its handwritten fixture, regenerate, verify the worktree is clean, and preserve
only the evidence—not the temporary product operation.
