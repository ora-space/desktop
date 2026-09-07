# specs Repository

`specs/` is an independent Git repository. Use `git -C specs` to inspect its status and history when
changing its contents. It primarily contains ADRs, core test cases, and domain documentation; see
`specs/AGENTS.md` for its detailed conventions.

# Rust/crates

1. **Code Documentation**: Unless it is a standard, self-explanatory method (e.g., `new()`), every function must include a comment above the signature describing its purpose. Provide inline comments for any complex logic, non-trivial algorithms, or specialized branching within function bodies. Write comments in English.
2. **Explain "Why", not "What"**: Use comments to explain design rationale, business logic constraints, or non-obvious trade-offs. Code structure and naming should inherently describe the "what."
3. **Design for Testability (DfT)**: Favor Dependency Injection and decoupled components. Define interfaces via Traits to allow easy mocking, and prefer small, pure functions that can be unit-tested in isolation.
4. **Prefer Static Dispatch**: Use Generics and Trait Bounds over Trait Objects (e.g., `Box<dyn Trait>`) to leverage monomorphization and compiler optimizations, unless runtime polymorphism is strictly necessary.
5. **Make Illegal States Unrepresentable**: Use Enums with associated data to model state machines, rather than Structs with many optional fields.
6. **Compatibility Boundaries**: Prioritize architectural cleanliness over preserving compatibility in code, public APIs, IPC/protocols, and user-facing behavior. Treat the local filesystem layout as a hard compatibility boundary: changes must not create path collisions or leave upgrades from older versions unable to start or operate safely. When changing the layout, detect, migrate, isolate, or otherwise safely handle existing files before using the new layout.

Ora is an IDE for AI Agent. In the crates folder where the rust code lives:

- Crate names are prefixed with `ora-`. For example, the `core` folder's crate is named `ora-core`
- Prefer `name.rs` as the module root with a sibling `name/` directory for submodules; do not introduce `mod.rs`.
- When using format! and you can inline variables into {}, always do that.
- Always collapse nested if statements which can be collapsed by &&-combining their conditions.
- Always inline format! args when possible.
- Use method references over closures which only invoke a method on the closure argument and can be replaced by referencing the method directly.
- Avoid bool or ambiguous `Option` parameters that force callers to write hard-to-read code such as `foo(false)` or `bar(None)`. Prefer enums, named methods, newtypes, or other idiomatic Rust API shapes when they keep the callsite self-documenting.
- When you cannot make that API change and still need a small positional-literal callsite in Rust, follow the `argument_comment_lint` convention:
  - Use an exact `/*param_name*/` comment before opaque literal arguments such as `None`, booleans, and numeric literals when passing them by position.
  - Do not add these comments for string or char literals unless the comment adds real clarity; those literals are intentionally exempt from the lint.
  - The parameter name in the comment must exactly match the callee signature.
- When possible, make `match` statements exhaustive and avoid wildcard arms.
- Never hardcode path separators or concatenate path strings manually. Always use `Path`, `PathBuf`, and `.join()` to construct and manipulate filesystem paths.
- Newly added traits should include doc comments that explain their role and how implementations are expected to use them.
- When writing tests, prefer comparing the equality of entire objects over fields one by one.
- When making a change that adds or changes behavior, ensure that the documentation in the `docs/` folder is up to date if applicable.
- Prefer private modules and explicitly exported public crate API.
- Do not create small helper methods that are referenced only once.
- Avoid large modules:
  - Prefer adding new modules instead of growing existing ones.
  - Target Rust modules under 500 LoC, excluding tests.
  - If a file exceeds roughly 800 LoC, add new functionality in a new module instead of extending
    the existing file unless there is a strong documented reason not to.
  - When extracting code from a large module, move the related tests and module/type docs toward
    the new implementation so the invariants stay close to the code that owns them.
- Use local time instead of UTC time.
- Use ora-logging wrapper macros instead of `tracing` macros. Use `ora_logging::clock::now_local` instead of `OffsetDateTime::now_local()`.
- Put logic that is generic — independent of any Ora domain concept, transport, or runtime — in `ora-utils` (`crates/utils`) instead of the calling crate. If you believe a piece of logic is generic, default to placing it in `ora-utils`. `ora-utils` must not depend on any other `ora-*` crate and must not carry domain vocabulary; gate heavier optional dependencies (such as archive formats) behind Cargo features so path-only consumers stay light.
- Before implementing path validation, normalization, or archive extraction, prefer the shared `ora-utils::path` and `ora-utils::archive` capabilities over crate-local logic. If `ora-utils` does not yet provide the required capability, extend `ora-utils` and then consume it instead of implementing it locally in the caller.

## Tests

`task test` runs the frontend and Rust workspace lint and test tasks. It can take a
long time, so prefer the smallest relevant task while iterating and run the full task
before considering a repository-wide change complete. Use `task --list` to see the
authoritative list of available tasks.

- Format changed files: `task format`
- Frontend lint: `task lint:frontend`
- Frontend tests: `task test:frontend`
- Rust workspace lint: `task lint:crates`
- Rust workspace tests: `task test:crates`
- All lint tasks: `task lint`
- All lint and test tasks (long-running): `task test`

### Test assertions

- Tests should use pretty_assertions::assert_eq for clearer diffs. Import this at the top of the test module if it isn't already.
- Prefer deep equals comparisons whenever possible. Perform `assert_eq!()` on entire objects, rather than individual fields.
- Avoid mutating process environment in tests; prefer passing environment-derived flags or dependencies from above.
- When testing structured events, logs, or spans using `tracing`, always install a test-scoped subscriber/dispatcher with an explicit `LevelFilter::TRACE` (or the required minimum level). Use `tracing::subscriber::with_default` or `tracing::dispatcher::with_default` to isolate the subscriber to the current test thread. Keep every operation that can emit the same `tracing` callsites under that scoped subscriber, including setup helpers, bootstrap code, repository fixtures, and API-surface smoke checks that create spans or events. This matters even for tests that do not assert logs directly: `tracing` caches callsite interest, so a normal test that touches a callsite first can make a later structured-log assertion fail intermittently. Prefer shared helpers such as `with_trace_logging` / `with_recorded_trace_logging` so ordinary tests and recording tests use the same scoped TRACE setup.

# TypeScript/packages

## Tests

Frontend package tests run under `scripts/run-with-clean-stderr.ts`. Any React Testing Library warning on stderr — especially `An update to … was not wrapped in act(...)` — fails the whole `task test` run even when Vitest reports green.

A test that renders anything calling `useTranslation` must import `appI18n` itself: react-i18next keeps its instance in a `node_modules` module that Vitest loads once per worker, so a file relying on an earlier file in the same worker to have initialized it passes locally and fails on CI, where a different worker split leaves it first and the missing-instance warning trips the clean-stderr gate on an otherwise green run.

When writing new frontend tests, fully await every operation that can update React: prefer awaited `userEvent` interactions and `findBy...` or `waitFor` assertions for user-visible outcomes; wrap direct writes to external stores such as Zustand in `act`; and handle promises, timers, animations, subscriptions, and editor transactions at their actual asynchronous boundary. Do not hide warnings with a global promise flush, arbitrary delay, or stderr allowlist. Otherwise an update can escape the test's `act` boundary or leak into a later test, producing stderr even though Vitest reports green and causing `task test` and CI to fail under `scripts/run-with-clean-stderr.ts`.
